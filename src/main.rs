mod board;
mod message;
mod session;
mod resources;

use hyper_util::rt::TokioIo;

use board::CellOwner;
use message::{GameMessageFactory, MessageType};
use session::{GameSession, GameSessionPhase};

use futures_channel::mpsc::unbounded;
use futures_util::{future, stream::TryStreamExt, StreamExt};
use std::convert::Infallible;
use std::task::Poll;
use std::{
    env,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use std::sync::atomic::{AtomicBool, Ordering};
use hyper::header::CONTENT_TYPE;
use hyper::{body::Incoming, header::{
    HeaderValue, CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION,
    UPGRADE,
}, server::conn::http1, service::service_fn, upgrade::Upgraded, HeaderMap, Method, Request, Response, StatusCode, Version};
use tokio_tungstenite::{
    tungstenite::{
        handshake::derive_accept_key,
        protocol::Role,
    },
    WebSocketStream,
};

use crate::message::message_send;
use crate::resources::StaticResource;
use tokio::net::TcpListener;

type PeerList = Arc<Mutex<Vec<Arc<Mutex<GameSession>>>>>;
type Body = http_body_util::Full<hyper::body::Bytes>;

#[tokio::main]
async fn main() -> Result<(), IoError> {
    let (js_socket_endpoint, listening_addr) = addresses();

    // Loads static resources only once
    let resources = StaticResource::new(&js_socket_endpoint).await;

    let game_sessions = PeerList::new(Mutex::new(Vec::with_capacity(6)));

    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind(&listening_addr).await;
    let listener = try_socket.expect("Failed to bind");
    println!("Listening on: {}", listening_addr);

    // Clean closed games job
    clean_closed_sessions(Arc::clone(&game_sessions));

    let game_message_factory = Arc::new(GameMessageFactory::new());

    // Handling each connection in a separate task.
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
    }

    Ok(())
}

fn addresses() -> (String, String) {
    let port = env::var("PORT")
        .unwrap_or_else(|_| String::from("8080"));
    let mut js_socket_endpoint = env::var("SOCKET_HOST")
        .unwrap_or_else(|_| {
            let mut fallback_endpoint = String::from("ws://127.0.0.1:");
            fallback_endpoint.push_str(&port);
            fallback_endpoint
        });
    js_socket_endpoint.push_str("/socket");
    let mut listening_addr = String::from("0.0.0.0:");
    listening_addr.push_str(&port);
    (js_socket_endpoint, listening_addr)
}

fn clean_closed_sessions(rc_game_sessions: Arc<Mutex<Vec<Arc<Mutex<GameSession>>>>>) {
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
}

async fn handle_request(
    mut req: Request<Incoming>,
    addr: SocketAddr,
    peer_list: PeerList,
    game_message_factory: Arc<GameMessageFactory>,
    resources: &'static StaticResource,
) -> Result<Response<Body>, Infallible> {
    let upgrade = HeaderValue::from_static("Upgrade");
    let websocket = HeaderValue::from_static("websocket");
    let headers = req.headers();
    let key = headers.get(SEC_WEBSOCKET_KEY);
    let derived = key.map(|k| derive_accept_key(k.as_bytes()));

    if is_not_socket_request(&req, &upgrade, headers, key)
    {
        handle_http_request(&req, resources)
    } else {
        println!("Received a new ws handshake");
        let ver = req.version();
        tokio::task::spawn(async move {
            match hyper::upgrade::on(&mut req).await {
                Ok(upgraded) => {
                    let upgraded = TokioIo::new(upgraded);
                    handle_websocket(
                        WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await,
                        addr,
                        peer_list,
                        game_message_factory,
                    )
                        .await;
                }
                Err(e) => println!("upgrade error: {}", e),
            }
        });
        websocket_handshake(ver, upgrade, websocket, derived)
    }
}

fn websocket_handshake(ver: Version, upgrade: HeaderValue, websocket: HeaderValue, derived: Option<String>) -> Result<Response<Body>, Infallible> {
    let mut res = Response::new(Body::default());
    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    *res.version_mut() = ver;
    res.headers_mut().append(CONNECTION, upgrade);
    res.headers_mut().append(UPGRADE, websocket);
    res.headers_mut().append(SEC_WEBSOCKET_ACCEPT, derived.unwrap().parse().unwrap());
    Ok(res)
}

fn is_not_socket_request(req: &Request<Incoming>, upgrade: &HeaderValue, headers: &HeaderMap, key: Option<&HeaderValue>) -> bool {
    req.method() != Method::GET
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
}

async fn handle_websocket(
    ws_stream: WebSocketStream<TokioIo<Upgraded>>,
    addr: SocketAddr,
    peer_list: PeerList,
    game_message_factory: Arc<GameMessageFactory>,
) {
    println!("WebSocket connection established: {}", addr);
    let active = AtomicBool::new(true);

    let (outgoing, incoming) = ws_stream.split();
    let (tx, rx) = unbounded();
    let tx = Arc::new(tx);
    let mut is_player_a = true;
    let gs = {
        let mut sessions = peer_list.lock().unwrap();
        match sessions.iter()
            .find(|s| { s.lock().unwrap().phase == GameSessionPhase::LOBBY }) {
            Some(&ref el) => {
                println!("Existing session found");
                let mut session = el.lock().unwrap();
                session.sender_b = Some(Arc::clone(&tx));
                session.start_game(&game_message_factory);
                is_player_a = false;
                Arc::clone(&el)
            }
            None => {
                println!("New session required");
                let out = Arc::new(Mutex::new(GameSession::new(Arc::clone(&tx))));
                sessions.push(Arc::clone(&out));
                message_send(&tx, game_message_factory.get_default(GameMessageFactory::WAITING_MESSAGE));
                out
            }
        }
    };
    let player = if is_player_a { CellOwner::PlayerA } else { CellOwner::PlayerB };

    let combined_input_output = {
        let input_processing = incoming
            .map_ok(|msg| { game_message_factory.parse_input(&msg) })
            .try_for_each(|input| {
                let mut game_session = gs.lock().unwrap();
                game_session.process_player_input(player, input, &game_message_factory);
                future::ok(())
            });

        let output_stream = rx
            .map(|msg| {
                if game_message_factory.parse_input(&msg).1 == MessageType::END {
                    active.store(false, Ordering::Relaxed);
                }
                msg
            })
            .take_until(future::poll_fn(|_| {
                if active.load(Ordering::Relaxed) {
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
    gs.close_session(player, &game_message_factory);
}

fn handle_http_request(req: &Request<Incoming>, resources: &'static StaticResource) -> Result<Response<Body>, Infallible> {
    if req.method() == Method::GET && req.uri() == "/app.js" {
        let mut res = Response::new(Body::from(&resources.javascript[..]));
        *res.status_mut() = StatusCode::OK;
        res.headers_mut().append(CONTENT_TYPE, "application/javascript".parse().unwrap());
        Ok(res)
    } else if req.method() == Method::GET && req.uri() == "/images/empty-cell.jpg" {
        let mut res = Response::new(Body::from(&resources.empty_cell[..]));
        *res.status_mut() = StatusCode::OK;
        res.headers_mut().append(CONTENT_TYPE, "image/jpeg".parse().unwrap());
        Ok(res)
    } else if req.method() == Method::GET && req.uri() == "/images/x-cell.jpg" {
        let mut res = Response::new(Body::from(&resources.x_cell[..]));
        *res.status_mut() = StatusCode::OK;
        res.headers_mut().append(CONTENT_TYPE, "image/jpeg".parse().unwrap());
        Ok(res)
    } else if req.method() == Method::GET && req.uri() == "/images/o-cell.jpg" {
        let mut res = Response::new(Body::from(&resources.o_cell[..]));
        *res.status_mut() = StatusCode::OK;
        res.headers_mut().append(CONTENT_TYPE, "image/jpeg".parse().unwrap());
        Ok(res)
    } else if req.method() == Method::GET && req.uri() == "/grid.css" {
        let mut res = Response::new(Body::from(&resources.css[..]));
        *res.status_mut() = StatusCode::OK;
        res.headers_mut().append(CONTENT_TYPE, "text/css".parse().unwrap());
        Ok(res)
    } else if req.method() == Method::GET && req.uri() == "/images/favicon.png" {
        let mut res = Response::new(Body::from(&resources.favicon[..]));
        *res.status_mut() = StatusCode::OK;
        res.headers_mut().append(CONTENT_TYPE, "image/png".parse().unwrap());
        Ok(res)
    } else {
        Ok(Response::new(Body::from(&resources.homepage[..])))
    }
}
