#[derive(PartialEq, Clone, Copy)]
pub enum CellOwner {
    None,
    PlayerA,
    PlayerB,
    Tie
}

impl CellOwner {

    pub fn opponent(&self) -> Self {
        match self {
            CellOwner::PlayerA => CellOwner::PlayerB,
            CellOwner::PlayerB => CellOwner::PlayerA,
            _ => CellOwner::None
        }
    }
}

pub struct GameBoard {
    cells: [CellOwner; 9],
}

impl GameBoard {
    pub fn new() -> GameBoard {
        GameBoard { cells: [CellOwner::None; 9] }
    }

    pub fn check_winner(&self) -> CellOwner {
        self.check_board_health();
        let checks: [CellOwner; 4] = [
            self.line_check(|i| i / 3), //rows
            self.line_check(|i| i % 3), //columns
            self.diagonal_check(|i| i % 4 == 0),
            self.diagonal_check(|i| i != 0 && i != 8 && i % 2 == 0)
        ];
        let winner = checks.iter()
            .find(|&&x| x == CellOwner::PlayerA || x == CellOwner::PlayerB);
        match winner {
            Some(&owner) => owner,
            None => self.cells.iter().find(|&&x| x == CellOwner::None)
                .map(|x| *x)
                .unwrap_or(CellOwner::Tie)
        }
    }

    pub fn update_cell(&mut self, index: usize, owner: CellOwner) -> bool {
        if self.cells[index] == CellOwner::None {
            self.cells[index] = owner;
            true
        } else {
            false
        }
    }

    fn check_board_health(&self) {
        let mut a_count: i32 = 0;
        let mut b_count: i32 = 0;
        for cell in self.cells {
            if cell == CellOwner::PlayerA {
                a_count += 1;
            } else if cell == CellOwner::PlayerB {
                b_count += 1;
            }
        }
        if (a_count - b_count).abs() >= 2 {
            panic!("Session corrupted: to many moves from one player")
        }
    }

    fn diagonal_check<F: Fn(usize) -> bool>(&self, filter: F) -> CellOwner {
        let mut a_count: i32 = 0;
        let mut b_count: i32 = 0;
        for (index, &owner) in self.cells.iter().enumerate() {
            if filter(index) {
                match owner {
                    CellOwner::PlayerA => a_count += 1,
                    CellOwner::PlayerB => b_count += 1,
                    _ => ()
                }
            }
        }
        if a_count == 3 {
            CellOwner::PlayerA
        } else if b_count == 3 {
            CellOwner::PlayerB
        } else {
            CellOwner::None
        }
    }

    fn line_check<F: Fn(usize) -> usize>(&self, transform: F) -> CellOwner {
        let mut a_counts = [0, 0, 0];
        let mut b_counts = [0, 0, 0];
        for (index, &owner) in self.cells.iter().enumerate() {
            let index = transform(index); // runtime upperbound of this operation is 2
            match owner {
                CellOwner::PlayerA => a_counts[index] += 1,
                CellOwner::PlayerB => b_counts[index] += 1,
                _ => ()
            }
        }
        if a_counts.iter().any(|&x| x == 3) {
            CellOwner::PlayerA
        } else if b_counts.iter().any(|&x| x == 3) {
            CellOwner::PlayerB
        } else { CellOwner::None }
    }
}
