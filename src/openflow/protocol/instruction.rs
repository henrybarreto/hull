use super::action::Action;
use super::constants::OFPIT_APPLY_ACTIONS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    ApplyActions(Vec<Action>),
}

impl Instruction {
    pub const fn apply_actions(actions: Vec<Action>) -> Self {
        Self::ApplyActions(actions)
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::ApplyActions(actions) => {
                let start = out.len();
                out.extend_from_slice(&OFPIT_APPLY_ACTIONS.to_be_bytes());
                out.extend_from_slice(&0u16.to_be_bytes());
                out.extend_from_slice(&[0u8; 4]);
                for action in actions {
                    action.encode(out);
                }
                let len = u16::try_from(out.len() - start).unwrap_or(u16::MAX);
                if let Some(dst) = out.get_mut(start + 2..start + 4) {
                    dst.copy_from_slice(&len.to_be_bytes());
                }
            }
        }
    }
}
