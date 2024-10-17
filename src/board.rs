

#[non_exhaustive]
pub struct CellOwner;

impl CellOwner {
    pub const NONE: &'static str = "NONE";
    pub const PLAYER_A: &'static str = "PLAYER_A";
    pub const PLAYER_B: &'static str = "PLAYER_B";
    pub const TIE: &'static str = "TIE";

    pub fn opponent(player: &str) -> &str  {
        match player {
            CellOwner::PLAYER_A => CellOwner::PLAYER_B,
            CellOwner::PLAYER_B => CellOwner::PLAYER_A,
            _ => CellOwner::NONE
        }
    }
}

pub struct GameBoard {
    cells: [&'static str; 9],
}

impl GameBoard {
    pub fn new() -> GameBoard {
        GameBoard{cells: [CellOwner::NONE; 9]}
    }

    pub fn check_winner(&self) -> &str {
        self.check_board_health();
        let checks: [&str; 4] = [
          self.line_check(|i| i/3), //rows
          self.line_check(|i| i % 3), //columns
          self.diagonal_check(|i| i % 4 == 0),
          self.diagonal_check(|i| i != 0 && i != 8  && i % 2 == 0)
        ];
        let winner = checks.iter()
            .find(|&&x| x == CellOwner::PLAYER_A || x == CellOwner::PLAYER_B);
        match winner {
            Some(owner) => *owner,
            None => self.cells.iter().find(|&&x| x == CellOwner::NONE)
                .map(|x| *x)
                .unwrap_or(CellOwner::TIE)
        }
    }

    pub fn update_cell(&mut self, index: usize, owner: &'static str) -> bool {
        if self.cells[index] == CellOwner::NONE {
            self.cells[index] = owner;
            true
        } else {
            false
        }
    }

    fn check_board_health(&self) {
        let mut a_count : i32 = 0;
        let mut b_count : i32 = 0;
        for cell in self.cells {
            if cell == CellOwner::PLAYER_A {
                a_count += 1;
            } else if cell == CellOwner::PLAYER_B {
                b_count += 1;
            }
        }
        if (a_count - b_count).abs() >= 2 {
            panic!("Session corrupted: to many moves from one player")
        }
    }

    fn diagonal_check<F : Fn(usize) -> bool>(&self, filter: F) -> &str {
        let mut a_count : i32 = 0;
        let mut b_count : i32 = 0;
        for (index, el) in self.cells.iter().enumerate() {
            let owner = *el;
            if filter(index) {
                match owner {
                    CellOwner::PLAYER_A => a_count += 1,
                    CellOwner::PLAYER_B => b_count += 1,
                    _ => ()
                }
            }
        }
        if a_count == 3 {
            CellOwner::PLAYER_A
        } else if b_count == 3 {
            CellOwner::PLAYER_B
        } else {
            CellOwner::NONE
        }
    }

    fn line_check<F: Fn(usize) -> usize>(&self, transform: F) -> &str {
        let mut a_counts = [0, 0, 0];
        let mut b_counts = [0, 0, 0];
        for (index, el) in self.cells.iter().enumerate() {
            let owner = *el;
            let index = transform(index); // runtime upperbound of this operation is 2
            match owner {
                CellOwner::PLAYER_A => a_counts[index] += 1,
                CellOwner::PLAYER_B => b_counts[index] += 1,
                _ => ()
            }
        }
        if a_counts.iter().any(|&x| x == 3) {
            CellOwner::PLAYER_A
        } else if b_counts.iter().any(|&x| x == 3) {
            CellOwner::PLAYER_B
        } else { CellOwner::NONE }
    }
}
