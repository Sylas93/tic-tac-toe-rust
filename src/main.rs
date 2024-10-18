//! A chat server that broadcasts a message to all connections.
//!
//! This is a simple line-based server which accepts WebSocket connections,
//! reads lines from those connections, and broadcasts the lines to all other
//! connected clients.
//!
//! You can test this out by running:
//!
//!     cargo run --example server 127.0.0.1:12345
//!
//! And then in another window run:
//!
//!     cargo run --example client ws://127.0.0.1:12345/
//!
//! You can run the second command in multiple windows and then chat between the
//! two, seeing the messages from the other client as they're received. For all
//! connected clients they'll all join the same room and see everyone else's
//! messages.

mod board;
mod message;

use board::{CellOwner, GameBoard};
use message::{GameMessageFactory, MessageType};

use std::{
    time::Duration,
    env,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use std::ops::Deref;
use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{future, pin_mut, stream::TryStreamExt, StreamExt};
use futures_util::task::SpawnExt;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::protocol::Message;

type PeerList = Arc<Mutex<Vec<GameSession>>>;

#[derive(PartialEq)]
enum GameSessionPhase {
    LOBBY,
    PLAYING,
    CLOSED
}

struct GameSession {
    board: GameBoard,
    phase: GameSessionPhase,
    turn: &'static str,
    sender_a: Option<(Arc<UnboundedSender<Message>>, SocketAddr)>,
    sender_b: Option<(Arc<UnboundedSender<Message>>, SocketAddr)>
}

impl GameSession {
    fn new() -> GameSession {
        GameSession {
            board: GameBoard::new(),
            phase: GameSessionPhase::LOBBY, turn: CellOwner::NONE, sender_a: None, sender_b: None
        }
    }

    fn start_game(&mut self, game_message_factory: &GameMessageFactory) {
        self.phase = GameSessionPhase::PLAYING;
        match &self.sender_a {
            Some(sender) => {
                sender.0.unbounded_send(
                    game_message_factory.get_default(GameMessageFactory::YOUR_TURN_MESSAGE)
                ).unwrap();
            },
            None => {println!("Non va start game A")}
        }
        match &self.sender_b {
            Some(sender) => {
                sender.0.unbounded_send(
                    game_message_factory.get_default(GameMessageFactory::OPPONENT_TURN_MESSAGE)
                ).unwrap();
            },
            None => {println!("Non va start game B")}
        }

    }

    fn update_board(&mut self, player: &'static str, message_text: &String, message_type: &String) -> bool {
        self.phase == GameSessionPhase::PLAYING &&
            self.turn == player &&
            message_type == MessageType::CLIENT_CLICK &&
            self.board.update_cell(message_text.parse().unwrap(), player)
    }
}

async fn handle_connection(raw_stream: TcpStream, addr: SocketAddr, peer_list: PeerList) {
    let game_message_factory = GameMessageFactory::new();
    let ws_stream = tokio_tungstenite::accept_async(raw_stream)
        .await
        .expect("Error during the websocket handshake occurred");
    println!("WebSocket connection established: {}", addr);

    let (outgoing, incoming) = ws_stream.split();
    let (tx, rx) = unbounded();
    let tx = Arc::new(tx);

    let result = async {
        let mut x = peer_list.lock().unwrap();
        let lobby_session = x
            .iter_mut()
            .filter(|s| s.phase == GameSessionPhase::LOBBY)
            .take(1).next();
        let result = match lobby_session {
            Some(lobby_session) => {
                println!("Found session");
                &tx.unbounded_send(
                    game_message_factory.build_message("You just joined an existing game!", MessageType::INFO)
                ).unwrap();
                lobby_session.sender_b = Some((Arc::clone(&tx), addr));
                lobby_session.start_game(&game_message_factory);

                println!("Session size: {}", x.len());
                (rx.map(Ok).forward(outgoing), CellOwner::PLAYER_B)
            },
            None => {
                println!("Creating session");
                let mut new_session = GameSession::new();
                &tx.unbounded_send(
                    game_message_factory.get_default(GameMessageFactory::WAITING_MESSAGE)
                ).unwrap();
                new_session.sender_a = Some((Arc::clone(&tx), addr));
                x.push(new_session);
                println!("Session size: {}", x.len());
                (rx.map(Ok).forward(outgoing), CellOwner::PLAYER_A)
            }
        };
        let (opponent_messages, player) = result;
        let input_messages = incoming.try_for_each(move |msg| {
            let (input_text, input_type) = game_message_factory.parse_input(msg);

            // echo server
            let mut echo_message = String::from("I got your message: ");
            echo_message.push_str(&*input_text);
            echo_message.push_str(player);
            &tx.unbounded_send(
                game_message_factory.build_message(&echo_message, MessageType::INFO)
            ).unwrap();
            future::ok(())
        });
        future::select(opponent_messages, input_messages)
    }.await;

    pin_mut!(result);

    result.await;

    println!("{} disconnected", &addr);

    let mut x = peer_list.lock().unwrap();
    let res = x.iter_mut().filter(|s| {
       let stored_addr = s.sender_a.as_ref().unwrap().1; //player A always exist
       println!("stored addr a is {}, against current addr {}", stored_addr, addr);
        stored_addr == addr || {
                match s.sender_b.as_ref() { Some(el) => {
                    let stored_addr = el.1;
                    println!("stored addr b is {}, against current addr {}", stored_addr, addr);
                    stored_addr == addr
                } , None => false }
        }
    }
    ).take(1).next();
    match res {
        Some(el) => {
            println!("Disconneted matched");
            el.phase = GameSessionPhase::CLOSED;
        },
        _ => println!("Not cleaned")
    }
}

#[tokio::main]
async fn main() -> Result<(), IoError> {
    let addr = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8080".to_string());

    let game_sessions = PeerList::new(Mutex::new(Vec::with_capacity(6)));
    let rc_game_sessions = Arc::clone(&game_sessions);
    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind(&addr).await;
    let listener = try_socket.expect("Failed to bind");
    println!("Listening on: {}", addr);

    // clean closed games
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(5000));
        loop {
            interval.tick().await;
            println!("tick");
            let mut indexes = Vec::new();
            let mut sessions = rc_game_sessions.lock().unwrap();
            for (index, el) in sessions.iter().enumerate() {
                if el.phase == GameSessionPhase::CLOSED {
                    indexes.push(index);
                }
            }
            for i in indexes {
                sessions.remove(i);
            }
        }
    });

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(handle_connection(stream, addr, game_sessions.clone()));
    }

    Ok(())
}
