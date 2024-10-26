mod board;
mod message;
mod session;
mod resources;

use hyper_util::rt::TokioIo;

use session::GameSession;
use board::{CellOwner};
use message::{GameMessageFactory, MessageType};

use std::{
    fs,
    time::Duration,
    env,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use std::convert::Infallible;
use std::task::Poll;
use futures_channel::mpsc::{unbounded, TrySendError, UnboundedSender};
use futures_util::{future, stream::TryStreamExt, StreamExt};

use hyper::{
    body::Incoming,
    header::{
        HeaderValue, CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION,
        UPGRADE,
    },
    server::conn::http1,
    service::service_fn,
    upgrade::Upgraded,
    Method, Request, Response, StatusCode, Version,
};
use hyper::header::CONTENT_TYPE;
use hyper::http::method::InvalidMethod;
use tokio_tungstenite::{
    tungstenite::{
        handshake::derive_accept_key,
        protocol::{Message, Role},
    },
    WebSocketStream,
};

use tokio::net::{TcpListener};
use crate::resources::StaticResource;

type PeerList = Arc<Mutex<Vec<Arc<Mutex<GameSession>>>>>;
type Body = http_body_util::Full<hyper::body::Bytes>;

#[derive(PartialEq)]
enum GameSessionPhase {
    LOBBY,
    PLAYING,
    CLOSED
}

fn message(plain: &str) -> Message {
    Message::Text(String::from(plain))
}

fn sent_fail_notify(_: TrySendError<Message>) {
    println!("Could not send message.")
}

fn multi_message_send(sender: &UnboundedSender<Message>, messages: &[&String]) {
    for &plain_message in messages {
        sender.unbounded_send(message(plain_message)).unwrap_or_else(sent_fail_notify);
    }
}

async fn handle_connection(
    ws_stream: WebSocketStream<TokioIo<Upgraded>>,
    addr: SocketAddr,
    peer_list: PeerList,
    game_message_factory: Arc<GameMessageFactory>
) {
    /*
    let ws_stream = tokio_tungstenite::accept_async(raw_stream)
        .await;
    if ws_stream.is_err() {
        println!("Error during the websocket handshake occurred");
        return;
    }
    let ws_stream = ws_stream.expect("Already verified");
    */
    println!("WebSocket connection established: {}", addr);
    let active = Arc::new(Mutex::new(true));

    let (outgoing, incoming) = ws_stream.split();
    let (tx, rx) = unbounded();
    let tx = Arc::new(tx);
    let mut is_player_a = true;
    let gs = {
        let mut sessions = peer_list.lock().unwrap();
        match sessions.iter()
        .find(|s| {s.lock().unwrap().phase == GameSessionPhase::LOBBY}) {
            Some(&ref el) => {
                println!("Existing session found");
                let mut session = el.lock().unwrap();
                session.sender_b = Some(Arc::clone(&tx));
                session.start_game(&game_message_factory);
                is_player_a = false;
                Arc::clone(&el)
            },
            None => {
                println!("New session required");
                let out = Arc::new(Mutex::new(GameSession::new(Arc::clone(&tx))));
                sessions.push(Arc::clone(&out));
                &tx.unbounded_send(
                    message(game_message_factory.get_default(GameMessageFactory::WAITING_MESSAGE))
                ).unwrap_or_else(sent_fail_notify);
                out
            }
        }
    };
    let player = if is_player_a {CellOwner::PLAYER_A} else {CellOwner::PLAYER_B};

    let combined_input_output = {
        let input_processing = incoming
            .map_ok(|msg|{ game_message_factory.parse_input(&msg)})
            .try_for_each(|input| {
               let mut game_session = gs.lock().unwrap();
                game_session.process_player_input(player, input, &game_message_factory);
                future::ok(())
            });

        let output_stream = rx
            .map(|msg| {
                if game_message_factory.parse_input(&msg).1 == MessageType::END {
                    *active.lock().unwrap() = false;
                }
                msg
            })
            .take_until(future::poll_fn(|_| {
                if *active.lock().unwrap() {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }))
            .map(Ok).forward(outgoing);
        future::select(output_stream, input_processing)
    };

    combined_input_output.await;

    println!("{} disconnected", &addr);

    let mut gs = gs.lock().unwrap();

    if gs.phase == GameSessionPhase::CLOSED {
        println!("Nothing to do, session already closed");
    } else {
        println!("Player let game before end");
        if gs.phase == GameSessionPhase::PLAYING { // if playing there must be an opponent, otherwise panic
            gs.opponent_sink(player)
                .unbounded_send(message(game_message_factory.get_default(GameMessageFactory::WITHDRAWAL_MESSAGE)))
                .unwrap_or_else(sent_fail_notify);
        }
        gs.phase = GameSessionPhase::CLOSED;
    }
}

#[tokio::main]
async fn main() -> Result<(), IoError> {
    let port = env::var("PORT")
        .unwrap_or_else(|_| String::from("8080"));
    let mut js_socket_endpoint = env::var("SOCKET_HOST")
        .unwrap_or_else(|_| {
            let mut fallback_endpoint = String::from("ws://127.0.0.1:");
            fallback_endpoint.push_str(&port);
            fallback_endpoint
        });
    js_socket_endpoint.push_str("/socket");
    let mut listening_addr = String::from("127.0.0.1:");
    listening_addr.push_str(&port);

    let resources = StaticResource::new(&js_socket_endpoint).await; // loads static resources only once

    let game_sessions = PeerList::new(Mutex::new(Vec::with_capacity(6)));
    let rc_game_sessions = Arc::clone(&game_sessions);
    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind(&listening_addr).await;
    let listener = try_socket.expect("Failed to bind");
    println!("Listening on: {}", listening_addr);

    // clean closed games
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(5000));
        loop {
            interval.tick().await;
            println!("tick");
            let mut indexes = Vec::new();
            let mut sessions = rc_game_sessions.lock().unwrap();
            println!("Before closed cleanup: {}", sessions.len());
            let mut counter = 0;
            for (index, el) in sessions.iter().enumerate() {
                if (*el).lock().unwrap().phase == GameSessionPhase::CLOSED {
                    indexes.push(index - counter);
                    counter += 1;
                }
            }
            for i in indexes {
                sessions.remove(i);
            }
            println!("After closed cleanup: {}", sessions.len());
        }
    });
    let game_message_factory = Arc::new(GameMessageFactory::new());
    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {

        let game_sessions = game_sessions.clone();
        let game_message_factory = Arc::clone(&game_message_factory); // other way of cloning

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let conn = http1::Builder::new()
                .serve_connection(io, service_fn(
                    move |req|
                        handle_request(req, addr, game_sessions.clone(), game_message_factory.clone(), resources)
                ))
                .with_upgrades();
            if let Err(err) = conn.await {
                eprintln!("failed to serve connection: {err:?}");
            }
        });
        //tokio::spawn(handle_connection(stream, addr, game_sessions.clone(), Arc::clone(&game_message_factory)));
    }

    Ok(())
}

async fn handle_request(
    mut req: Request<Incoming>,
    addr: SocketAddr,
    peer_list: PeerList,
    game_message_factory: Arc<GameMessageFactory>,
    resources: &'static StaticResource
) -> Result<Response<Body>, Infallible> {
    println!("The request's path is: {}", req.uri().path());
    println!("The request's headers are:");
    for (ref header, _value) in req.headers() {
        println!("* {}", header);
    }
    let upgrade = HeaderValue::from_static("Upgrade");
    let websocket = HeaderValue::from_static("websocket");
    let headers = req.headers();
    let key = headers.get(SEC_WEBSOCKET_KEY);
    let derived = key.map(|k| derive_accept_key(k.as_bytes()));
    if req.method() != Method::GET
        || req.version() < Version::HTTP_11
        || !headers
        .get(CONNECTION)
        .and_then(|h| h.to_str().ok())
        .map(|h| {
            h.split(|c| c == ' ' || c == ',')
                .any(|p| p.eq_ignore_ascii_case(upgrade.to_str().unwrap()))
        })
        .unwrap_or(false)
        || !headers
        .get(UPGRADE)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
        || !headers.get(SEC_WEBSOCKET_VERSION).map(|h| h == "13").unwrap_or(false)
        || key.is_none()
        || req.uri() != "/socket"
    {
        if req.method() == Method::GET && req.uri() == "/app.js" {
            let mut res = Response::new(Body::from(&resources.javascript[..]));
            *res.status_mut() = StatusCode::OK;
            res.headers_mut().append(CONTENT_TYPE, "application/javascript".parse().unwrap());
            return Ok(res);
        } else if req.method() == Method::GET && req.uri() == "/images/empty-cell.jpg" {
            let mut res = Response::new(Body::from(&resources.empty_cell[..]));
            *res.status_mut() = StatusCode::OK;
            res.headers_mut().append(CONTENT_TYPE, "image/jpeg".parse().unwrap());
            return Ok(res);
        } else if req.method() == Method::GET && req.uri() == "/images/x-cell.jpg" {
            let mut res = Response::new(Body::from(&resources.x_cell[..]));
            *res.status_mut() = StatusCode::OK;
            res.headers_mut().append(CONTENT_TYPE, "image/jpeg".parse().unwrap());
            return Ok(res);
        } else if req.method() == Method::GET && req.uri() == "/images/o-cell.jpg" {
            let mut res = Response::new(Body::from(&resources.o_cell[..]));
            *res.status_mut() = StatusCode::OK;
            res.headers_mut().append(CONTENT_TYPE, "image/jpeg".parse().unwrap());
            return Ok(res);
        } else if req.method() == Method::GET && req.uri() == "/grid.css" {
            let mut res = Response::new(Body::from(&resources.css[..]));
            *res.status_mut() = StatusCode::OK;
            res.headers_mut().append(CONTENT_TYPE, "text/css".parse().unwrap());
            return Ok(res);
        } else if req.method() == Method::GET && req.uri() == "/images/favicon.png" {
            let mut res = Response::new(Body::from(&resources.favicon[..]));
            *res.status_mut() = StatusCode::OK;
            res.headers_mut().append(CONTENT_TYPE, "image/png".parse().unwrap());
            return Ok(res);
        } else {
            return Ok(Response::new(Body::from(&resources.homepage[..])));
        };
    }

    println!("Received a new ws handshake");
    let ver = req.version();
    tokio::task::spawn(async move {
        match hyper::upgrade::on(&mut req).await {
            Ok(upgraded) => {
                let upgraded = TokioIo::new(upgraded);
                handle_connection(
                    WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await,
                    addr,
                    peer_list,
                    game_message_factory
                )
                    .await;
            }
            Err(e) => println!("upgrade error: {}", e),
        }
    });
    let mut res = Response::new(Body::default());
    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    *res.version_mut() = ver;
    res.headers_mut().append(CONNECTION, upgrade);
    res.headers_mut().append(UPGRADE, websocket);
    res.headers_mut().append(SEC_WEBSOCKET_ACCEPT, derived.unwrap().parse().unwrap());
    // Let's add an additional header to our response to the client.
    res.headers_mut().append("MyCustomHeader", ":)".parse().unwrap());
    res.headers_mut().append("SOME_TUNGSTENITE_HEADER", "header_value".parse().unwrap());
    Ok(res)
}

static NOTFOUND: &[u8] = b"Not Found";


