//! Input actions for injecting into egui.
#![allow(missing_docs)]

use std::{array, collections::HashMap, sync::Mutex};

use crate::{
    registry::lock,
    types::{Modifiers, Pos2, Vec2},
};

#[derive(Debug, Clone)]
pub enum InputAction {
    PointerMove {
        pos: Pos2,
    },
    PointerButton {
        pos: Pos2,
        button: egui::PointerButton,
        pressed: bool,
        modifiers: Modifiers,
    },
    Key {
        key: egui::Key,
        pressed: bool,
        modifiers: Modifiers,
    },
    Text {
        text: String,
    },
    Paste {
        text: String,
    },
    Scroll {
        delta: Vec2,
        modifiers: Modifiers,
    },
}

impl InputAction {
    pub fn apply(self, raw_input: &mut egui::RawInput) {
        match self {
            Self::PointerMove { pos } => {
                raw_input.events.push(egui::Event::PointerMoved(pos.into()));
            }
            Self::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } => {
                raw_input.events.push(egui::Event::PointerButton {
                    pos: pos.into(),
                    button,
                    pressed,
                    modifiers: modifiers.into(),
                });
            }
            Self::Key {
                key,
                pressed,
                modifiers,
            } => {
                raw_input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: modifiers.into(),
                });
            }
            Self::Text { text } => {
                raw_input.events.push(egui::Event::Text(text));
            }
            Self::Paste { text } => {
                raw_input.events.push(egui::Event::Paste(text));
            }
            Self::Scroll { delta, modifiers } => {
                raw_input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: delta.into(),
                    modifiers: modifiers.into(),
                });
            }
        }
    }
}

const ACTION_STAGE_COUNT: usize = 3;

type ActionMap = HashMap<egui::ViewportId, Vec<InputAction>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTiming {
    Current,
    Next,
    AfterNext,
}

impl ActionTiming {
    fn index(self) -> usize {
        match self {
            Self::Current => 0,
            Self::Next => 1,
            Self::AfterNext => 2,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Current => "actions lock",
            Self::Next => "actions next lock",
            Self::AfterNext => "actions next next lock",
        }
    }
}

pub struct ActionQueue {
    staged_actions: [Mutex<ActionMap>; ACTION_STAGE_COUNT],
    commands: Mutex<HashMap<egui::ViewportId, Vec<egui::ViewportCommand>>>,
}

impl Default for ActionQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionQueue {
    pub fn new() -> Self {
        Self {
            staged_actions: array::from_fn(|_| Mutex::new(HashMap::new())),
            commands: Mutex::new(HashMap::new()),
        }
    }

    pub fn queue_action_with_timing(
        &self,
        viewport_id: egui::ViewportId,
        timing: ActionTiming,
        action: InputAction,
    ) {
        let queue = &self.staged_actions[timing.index()];
        queue_to_map(queue, timing.label(), viewport_id, action);
    }

    pub fn queue_command(&self, viewport_id: egui::ViewportId, command: egui::ViewportCommand) {
        queue_to_map(&self.commands, "commands lock", viewport_id, command);
    }

    pub fn drain_actions(&self, viewport_id: egui::ViewportId) -> Vec<InputAction> {
        let current = self.take_staged_actions(ActionTiming::Current, viewport_id);
        self.promote_staged_actions(ActionTiming::Current, ActionTiming::Next, viewport_id);
        self.promote_staged_actions(ActionTiming::Next, ActionTiming::AfterNext, viewport_id);
        current
    }

    pub fn drain_all_commands(&self) -> Vec<(egui::ViewportId, Vec<egui::ViewportCommand>)> {
        let mut commands = lock(&self.commands, "commands lock");
        commands.drain().collect()
    }

    pub fn clear_all(&self) {
        for timing in [
            ActionTiming::Current,
            ActionTiming::Next,
            ActionTiming::AfterNext,
        ] {
            lock(&self.staged_actions[timing.index()], timing.label()).clear();
        }
        lock(&self.commands, "commands lock").clear();
    }

    pub fn has_pending_actions(&self, viewport_id: egui::ViewportId) -> bool {
        [
            ActionTiming::Current,
            ActionTiming::Next,
            ActionTiming::AfterNext,
        ]
        .into_iter()
        .any(|timing| {
            has_pending(
                &self.staged_actions[timing.index()],
                timing.label(),
                viewport_id,
            )
        })
    }

    pub fn has_pending_commands(&self, viewport_id: egui::ViewportId) -> bool {
        has_pending(&self.commands, "commands lock", viewport_id)
    }

    fn take_staged_actions(
        &self,
        timing: ActionTiming,
        viewport_id: egui::ViewportId,
    ) -> Vec<InputAction> {
        let mut queue = lock(&self.staged_actions[timing.index()], timing.label());
        queue.remove(&viewport_id).unwrap_or_default()
    }

    fn promote_staged_actions(
        &self,
        target: ActionTiming,
        source: ActionTiming,
        viewport_id: egui::ViewportId,
    ) {
        let next_actions = self.take_staged_actions(source, viewport_id);
        if next_actions.is_empty() {
            return;
        }
        let mut queue = lock(&self.staged_actions[target.index()], target.label());
        queue.entry(viewport_id).or_default().extend(next_actions);
    }
}

fn queue_to_map<T>(
    queue: &Mutex<HashMap<egui::ViewportId, Vec<T>>>,
    label: &'static str,
    viewport_id: egui::ViewportId,
    value: T,
) {
    let mut queue = lock(queue, label);
    queue.entry(viewport_id).or_default().push(value);
}

fn has_pending<T>(
    queue: &Mutex<HashMap<egui::ViewportId, Vec<T>>>,
    label: &'static str,
    viewport_id: egui::ViewportId,
) -> bool {
    let queue = lock(queue, label);
    queue
        .get(&viewport_id)
        .is_some_and(|items| !items.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_payload(action: InputAction) -> String {
        match action {
            InputAction::Text { text } => text,
            other => panic!("expected text action, got {other:?}"),
        }
    }

    #[test]
    fn drain_actions_promotes_staged_actions_one_frame_at_a_time() {
        let queue = ActionQueue::new();
        let viewport_id = egui::ViewportId::ROOT;

        queue.queue_action_with_timing(
            viewport_id,
            ActionTiming::Current,
            InputAction::Text {
                text: "current".to_string(),
            },
        );
        queue.queue_action_with_timing(
            viewport_id,
            ActionTiming::Next,
            InputAction::Text {
                text: "next".to_string(),
            },
        );
        queue.queue_action_with_timing(
            viewport_id,
            ActionTiming::AfterNext,
            InputAction::Text {
                text: "later".to_string(),
            },
        );

        let current = queue
            .drain_actions(viewport_id)
            .into_iter()
            .map(text_payload)
            .collect::<Vec<_>>();
        let next = queue
            .drain_actions(viewport_id)
            .into_iter()
            .map(text_payload)
            .collect::<Vec<_>>();
        let later = queue
            .drain_actions(viewport_id)
            .into_iter()
            .map(text_payload)
            .collect::<Vec<_>>();

        assert_eq!(current, vec!["current".to_string()]);
        assert_eq!(next, vec!["next".to_string()]);
        assert_eq!(later, vec!["later".to_string()]);
        assert!(!queue.has_pending_actions(viewport_id));
    }
}
