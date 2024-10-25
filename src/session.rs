use std::sync::{Arc};
use futures_channel::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::Message;
use crate::board::{CellOwner, GameBoard};
use crate::{message, multi_message_send, sent_fail_notify, GameSessionPhase};
use crate::message::{GameMessageFactory, MessageType};

pub struct GameSession {
    pub(crate) board: GameBoard,
    pub(crate) phase: GameSessionPhase,
    pub(crate) turn: &'static str,
    sender_a: Arc<UnboundedSender<Message>>,
    pub(crate) sender_b: Option<Arc<UnboundedSender<Message>>>
}

impl GameSession {
    pub fn new(sender_a: Arc<UnboundedSender<Message>>) -> GameSession {
        GameSession {
            board: GameBoard::new(),
            phase: GameSessionPhase::LOBBY, turn: CellOwner::PLAYER_A, sender_a, sender_b: None
        }
    }

    pub fn start_game(&mut self, game_message_factory: &GameMessageFactory) {
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

    pub fn process_player_input(
        &mut self,
        player: &'static str,
        (input_text, input_type): (String, String),
        game_message_factory: &GameMessageFactory
    ) {
        if self.update_board(player, &input_text, &input_type) {
            println!("Board updated!");
            let figure_message = if player == CellOwner::PLAYER_A {
                game_message_factory.get_default(GameMessageFactory::X_FIGURE_MESSAGE)
            } else {
                game_message_factory.get_default(GameMessageFactory::O_FIGURE_MESSAGE)
            };
            let show_message = &GameMessageFactory::build_plain_message(&input_text, MessageType::SHOW);
            let winner = self.board.check_winner();
            if winner == CellOwner::NONE {
                self.turn = CellOwner::opponent(self.turn);
                multi_message_send(
                    &self.opponent_sink(player),
                    &[figure_message, show_message,
                        game_message_factory.get_default(GameMessageFactory::YOUR_TURN_MESSAGE)]
                );
                multi_message_send(
                    &self.opponent_sink(CellOwner::opponent(player)),
                    &[figure_message, show_message,
                        game_message_factory.get_default(GameMessageFactory::OPPONENT_TURN_MESSAGE)]
                );
            } else if winner == player {
                self.phase = GameSessionPhase::CLOSED;
                multi_message_send(
                    &self.opponent_sink(player),
                    &[figure_message, show_message,
                        game_message_factory.get_default(GameMessageFactory::LOST_MESSAGE)]
                );
                multi_message_send(
                    &self.opponent_sink(CellOwner::opponent(player)),
                    &[figure_message, show_message,
                        game_message_factory.get_default(GameMessageFactory::WIN_MESSAGE)]
                );
            } else if winner == CellOwner::TIE {
                self.phase = GameSessionPhase::CLOSED;
                let tie_messages = &[figure_message, show_message,
                    game_message_factory.get_default(GameMessageFactory::TIE_MESSAGE)];
                multi_message_send(
                    &*self.opponent_sink(player), tie_messages);
                multi_message_send(
                    &*self.opponent_sink(CellOwner::opponent(player)), tie_messages);
            }
        }
    }

    pub fn opponent_sink(&self, player: &str) -> Arc<UnboundedSender<Message>> {
        if player == CellOwner::PLAYER_A {
            self.sender_b.as_ref().unwrap().clone()
        } else {
            self.sender_a.clone()
        }
    }

    fn update_board(&mut self, player: &'static str, message_text: &String, message_type: &String) -> bool {
        self.phase == GameSessionPhase::PLAYING &&
            self.turn == player &&
            message_type == MessageType::CLIENT_CLICK &&
            self.board.update_cell(message_text.parse().unwrap(), player)
    }
}
