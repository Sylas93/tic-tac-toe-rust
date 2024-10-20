use std::collections::HashMap;
use tokio_tungstenite::tungstenite::protocol::Message;
use serde_json::{Value::String as JsonString, Value};

#[non_exhaustive]
pub struct MessageType;

impl MessageType {
    pub const CLIENT_CLICK: &'static str = "CLIENT_CLICK";
    pub const SHOW: &'static str = "SHOW";
    pub const INFO: &'static str = "INFO";
    pub const ERROR: &'static str = "ERROR";
    pub const FIGURE: &'static str = "FIGURE";
    pub const END: &'static str = "END";
}

pub struct GameMessageFactory{
    defaults: HashMap<usize, String>
}

impl GameMessageFactory {

    pub const YOUR_TURN_MESSAGE: usize = 0;
    pub const OPPONENT_TURN_MESSAGE: usize = 1;
    pub const WAITING_MESSAGE: usize = 2;
    pub const LOST_MESSAGE: usize = 3;
    pub const WIN_MESSAGE: usize = 4;
    pub const TIE_MESSAGE: usize = 5;
    pub const WITHDRAWAL_MESSAGE: usize = 6;
    pub const X_FIGURE_MESSAGE: usize  = 7;
    pub const O_FIGURE_MESSAGE: usize = 8;

    pub fn new() -> GameMessageFactory {
        let defaults = HashMap::from([
            (Self::YOUR_TURN_MESSAGE, Self::build_plain_message("Your turn!", MessageType::INFO)),
            (Self::OPPONENT_TURN_MESSAGE, Self::build_plain_message("Opponent turn!", MessageType::INFO)),
            (Self::WAITING_MESSAGE, Self::build_plain_message("Waiting for opponent", MessageType::INFO)),
            (Self::LOST_MESSAGE, Self::build_plain_message("You lost!<br><br>Tap here to play again!", MessageType::END)),
            (Self::WIN_MESSAGE, Self::build_plain_message("You won!<br><br>Tap here to play again!", MessageType::END)),
            (Self::TIE_MESSAGE, Self::build_plain_message("Tie!<br><br>Tap here to play again!", MessageType::END)),
            (Self::WITHDRAWAL_MESSAGE, Self::build_plain_message("Your opponent left the game!<br><br>Tap here to play again!", MessageType::END)),
            (Self::X_FIGURE_MESSAGE, Self::build_plain_message("x-cell", MessageType::FIGURE )),
            (Self::O_FIGURE_MESSAGE, Self::build_plain_message("o-cell", MessageType::FIGURE ))
        ]);

        GameMessageFactory {
            defaults
        }
    }

    pub fn get_default(&self, index: usize) -> &String {
        match self.defaults.get(&index) {
            Some(value) => value,
            None => panic!("No default value found for index {}", index)
        }
    }

    pub fn parse_input(&self, input: &Message) -> (String, String) {
        let input_text = input.to_text().unwrap();
        println!("Received a message: {}", input_text);
        let v: Value = serde_json::from_str(input_text).unwrap();
        let input_text = match v.get("text").unwrap() {
            JsonString(str) => String::from(str),
            _ => panic!("Invalid text from client")
        };
        let input_type= match v.get("type").unwrap() {
            JsonString(str) =>  String::from(str),
            _ => panic!("Invalid type from client")
        };
        (input_text, input_type)
    }

    pub fn build_plain_message(m: &str, t: &str) -> String {
        let template = "{ \"text\": \"$\", \"type\": \"$\" }";
        let message = String::from(template).replacen("$", m, 1).replacen("$", t, 1);
        println!("The resulting message is: {message}");
        message
    }
}
