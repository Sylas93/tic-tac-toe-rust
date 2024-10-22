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
use std::task::Poll;
use futures_channel::mpsc::{unbounded, TrySendError, UnboundedSender};
use futures_util::{future, stream::TryStreamExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::protocol::Message;

type PeerList = Arc<Mutex<Vec<Arc<Mutex<GameSession>>>>>;

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

struct GameSession {
    board: GameBoard,
    phase: GameSessionPhase,
    turn: &'static str,
    sender_a: Arc<UnboundedSender<Message>>,
    sender_b: Option<Arc<UnboundedSender<Message>>>
}

impl GameSession {
    fn new(sender_a: Arc<UnboundedSender<Message>>) -> GameSession {
        GameSession {
            board: GameBoard::new(),
            phase: GameSessionPhase::LOBBY, turn: CellOwner::PLAYER_A, sender_a, sender_b: None
        }
    }

    fn start_game(&mut self, game_message_factory: &GameMessageFactory) {
        println!("Starting game");
        self.phase = GameSessionPhase::PLAYING;
        self.sender_a.unbounded_send(
            message(game_message_factory.get_default(GameMessageFactory::YOUR_TURN_MESSAGE))
        ).unwrap_or_else(sent_fail_notify);
        match &self.sender_b {
            Some(sender) => {
                sender.unbounded_send(
                    message(game_message_factory.get_default(GameMessageFactory::OPPONENT_TURN_MESSAGE))
                ).unwrap_or_else(sent_fail_notify);
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

    fn opponent_sink(&self, player: &str) -> Arc<UnboundedSender<Message>> {
        if player == CellOwner::PLAYER_A {
            self.sender_b.as_ref().unwrap().clone()
        } else {
            self.sender_a.clone()
        }
    }
}

async fn handle_connection(
    raw_stream: TcpStream,
    addr: SocketAddr,
    peer_list: PeerList,
    game_message_factory: GameMessageFactory
) {
    let ws_stream = tokio_tungstenite::accept_async(raw_stream)
        .await
        .expect("Error during the websocket handshake occurred");
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
                player_feedback(player, input, &game_message_factory, Arc::clone(&gs));
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

fn player_feedback(
    player: &'static str,
    (input_text, input_type): (String, String),
    game_message_factory: &GameMessageFactory,
    game_session: Arc<Mutex<GameSession>>
) {
    let mut game_session = game_session.lock().unwrap();
    if (*game_session).update_board(player, &input_text, &input_type) {
        println!("Board updated!");
        let figure_message = if player == CellOwner::PLAYER_A {
            game_message_factory.get_default(GameMessageFactory::X_FIGURE_MESSAGE)
        } else {
            game_message_factory.get_default(GameMessageFactory::O_FIGURE_MESSAGE)
        };
        let show_message = &GameMessageFactory::build_plain_message(&input_text, MessageType::SHOW);
        let winner = game_session.board.check_winner();
        if winner == CellOwner::NONE {
            game_session.turn = CellOwner::opponent(game_session.turn);
            multi_message_send(
                &*game_session.opponent_sink(player),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::YOUR_TURN_MESSAGE)]
            );
            multi_message_send(
                &*game_session.opponent_sink(CellOwner::opponent(player)),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::OPPONENT_TURN_MESSAGE)]
            );
        } else if winner == player {
            game_session.phase = GameSessionPhase::CLOSED;
            multi_message_send(
                &*game_session.opponent_sink(player),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::LOST_MESSAGE)]
            );
            multi_message_send(
                &*game_session.opponent_sink(CellOwner::opponent(player)),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::WIN_MESSAGE)]
            );
        } else if winner == CellOwner::TIE {
            game_session.phase = GameSessionPhase::CLOSED;
            let tie_messages = &[figure_message, show_message,
                game_message_factory.get_default(GameMessageFactory::TIE_MESSAGE)];
            multi_message_send(
                &*game_session.opponent_sink(player), tie_messages);
            multi_message_send(
                &*game_session.opponent_sink(CellOwner::opponent(player)), tie_messages);
        }
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
            println!("Before closed cleanup: {}", sessions.len());
            for (index, el) in sessions.iter().enumerate() {
                if (*el).lock().unwrap().phase == GameSessionPhase::CLOSED {
                    indexes.push(index);
                }
            }
            for i in indexes {
                sessions.remove(i);
            }
            println!("After closed cleanup: {}", sessions.len());
        }
    });

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(handle_connection(stream, addr, game_sessions.clone(), GameMessageFactory::new()));
    }

    Ok(())
}
