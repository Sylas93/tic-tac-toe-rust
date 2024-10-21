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
use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{future, pin_mut, stream::TryStreamExt, StreamExt};
use futures_util::task::SpawnExt;
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

fn multi_message_send(sender: &UnboundedSender<Message>, messages: &[&String]) {
    for &plain_message in messages {
        sender.unbounded_send(message(plain_message)).expect("Failed to send message");
    }
}

struct GameSession {
    board: GameBoard,
    phase: GameSessionPhase,
    turn: &'static str,
    sender_a: Option<(Arc<UnboundedSender<Message>>, SocketAddr)>,
    sender_b: Option<(Arc<UnboundedSender<Message>>, SocketAddr)>,
}

impl GameSession {
    fn new(sender_a: Option<(Arc<UnboundedSender<Message>>, SocketAddr)>) -> GameSession {
        GameSession {
            board: GameBoard::new(),
            phase: GameSessionPhase::LOBBY, turn: CellOwner::PLAYER_A, sender_a, sender_b: None
        }
    }

    fn start_game(&mut self, game_message_factory: &GameMessageFactory) {
        println!("Starting game");
        self.phase = GameSessionPhase::PLAYING;
        match &self.sender_a {
            Some(sender) => {
                sender.0.unbounded_send(
                    message(game_message_factory.get_default(GameMessageFactory::YOUR_TURN_MESSAGE))
                ).unwrap();
            },
            None => {println!("Non va start game A")}
        }
        match &self.sender_b {
            Some(sender) => {
                sender.0.unbounded_send(
                    message(game_message_factory.get_default(GameMessageFactory::OPPONENT_TURN_MESSAGE))
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

    fn opponent_sink(&self, player: &str) -> Arc<UnboundedSender<Message>> {
        if player == CellOwner::PLAYER_A {
            self.sender_b.as_ref().unwrap().0.clone()
        } else {
            self.sender_a.as_ref().unwrap().0.clone()
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
    let active1 = Arc::clone(&active);

    let (outgoing, incoming) = ws_stream.split();
    let (tx, rx) = unbounded();
    let tx = Arc::new(tx);
    let mut is_player_a = true;
    let gs = {
        let x = Arc::clone(&peer_list);
        let mut x = x.lock().unwrap();
        match x.iter()
        .find(|s| {s.lock().unwrap().phase == GameSessionPhase::LOBBY}) {
            Some(&ref el) => {
                println!("Existing session found");
                let mut session = el.lock().unwrap();
                session.sender_b = Some((Arc::clone(&tx), addr));
                session.start_game(&game_message_factory);
                is_player_a = false;
                Arc::clone(&el)
            },
            None => {
                println!("New session required");
                let out = Arc::new(Mutex::new(GameSession::new(Some((Arc::clone(&tx), addr)))));
                x.push(Arc::clone(&out));
                println!("Deadlock");
                &tx.unbounded_send(
                    message(game_message_factory.get_default(GameMessageFactory::WAITING_MESSAGE))
                ).unwrap();
                println!("Completed None");
                out
            }
        }
    };

    let result = async {

        let player =  if is_player_a {CellOwner::PLAYER_A} else {CellOwner::PLAYER_B};

        let feedback = incoming
            .try_take_while(|_| { future::ok(
                *active.lock().unwrap()
            )})
            .map_ok(|msg|{ game_message_factory.parse_input(&msg)})
            //.try_take_while(|(_, input_type)| {future::ok(input_type != MessageType::END)})
            .try_for_each(|input| {
                player_feedback(player, input, &game_message_factory, Arc::clone(&gs));
                future::ok(())
            });

        let opponent_messages = rx
            .map(|x| {
                if game_message_factory.parse_input(&x).1 == MessageType::END {
                    *active1.lock().unwrap() = false;
                }
                x
            })
            //.peekable()
            //.peek()
            //.take_while(async {|msg| {future::ready(game_message_factory.parse_input(msg).1 == MessageType::END)}})
            .map(Ok).forward(outgoing);
        future::select(opponent_messages, feedback)
    }.await;

    pin_mut!(result);

    result.await;

    println!("{} disconnected", &addr);

    let mut x = peer_list.lock().unwrap();
    let res = x.iter_mut().filter(|s| {
       let stored_addr = s.lock().unwrap().sender_a.as_ref().unwrap().1; //player A always exist
       println!("stored addr a is {}, against current addr {}", stored_addr, addr);
        stored_addr == addr || {
                match s.lock().unwrap().sender_b.as_ref() { Some(el) => {
                    let stored_addr = el.1;
                    println!("stored addr b is {}, against current addr {}", stored_addr, addr);
                    stored_addr == addr
                } , None => false }
        }
    }
    ).take(1).next();
    match res {
        Some(el) => {
            println!("Disconnected matched");
            el.lock().unwrap().phase = GameSessionPhase::CLOSED;
        },
        _ => println!("Not cleaned")
    }
}

fn player_feedback(
    player: &'static str,
    (input_text, input_type): (String, String),
    game_message_factory: &GameMessageFactory,
    lobby_session: Arc<Mutex<GameSession>>
) {
    let mut lobby_session = lobby_session.lock().unwrap();
    if (*lobby_session).update_board(player, &input_text, &input_type) {
        println!("Board updated!");
        let figure_message = if player == CellOwner::PLAYER_A {
            game_message_factory.get_default(GameMessageFactory::X_FIGURE_MESSAGE)
        } else {
            game_message_factory.get_default(GameMessageFactory::O_FIGURE_MESSAGE)
        };
        let show_message = &GameMessageFactory::build_plain_message(&input_text, MessageType::SHOW);
        let winner = lobby_session.board.check_winner();
        if winner == CellOwner::NONE {
            lobby_session.turn = CellOwner::opponent(lobby_session.turn);
            multi_message_send(
                &*lobby_session.opponent_sink(player),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::YOUR_TURN_MESSAGE)]
            );
            multi_message_send(
                &*lobby_session.opponent_sink(CellOwner::opponent(player)),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::OPPONENT_TURN_MESSAGE)]
            );
        } else if winner == player {
            lobby_session.phase = GameSessionPhase::CLOSED;
            multi_message_send(
                &*lobby_session.opponent_sink(player),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::LOST_MESSAGE)]
            );
            multi_message_send(
                &*lobby_session.opponent_sink(CellOwner::opponent(player)),
                &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::WIN_MESSAGE)]
            );
        } else if winner == CellOwner::TIE {
            lobby_session.phase = GameSessionPhase::CLOSED;
            let tie_messages = &[figure_message, show_message,
                game_message_factory.get_default(GameMessageFactory::TIE_MESSAGE)];
            multi_message_send(
                &*lobby_session.opponent_sink(player), tie_messages);
            multi_message_send(
                &*lobby_session.opponent_sink(CellOwner::opponent(player)), tie_messages);
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
