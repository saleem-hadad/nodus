//! The board's undo history for what's drawn on it: ink, shapes and links
//! (⌘Z, ⇧⌘Z; INKUI-021, SHAPE-040). Ink goes back as whole lists of
//! strokes, as 0.9.4's did. Shapes and links go back one by one, each as
//! it was before and after the change, so a shape a session draws in the
//! meantime is left alone.

use nodus_domain::{BoardLink, BoardShape, InkStroke};

use crate::ink_draw::HISTORY;

/// Shapes and links a change touched: each as it was, and as it became
/// (none where it wasn't, or went).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub shapes: Vec<(Option<BoardShape>, Option<BoardShape>)>,
    pub links: Vec<(Option<BoardLink>, Option<BoardLink>)>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.links.is_empty()
    }

    /// The same change the other way round.
    pub fn reversed(&self) -> Self {
        Self {
            shapes: self.shapes.iter().map(|(a, b)| (b.clone(), a.clone())).collect(),
            links: self.links.iter().map(|(a, b)| (b.clone(), a.clone())).collect(),
        }
    }

    /// Shapes added or changed.
    pub fn shape(&mut self, before: Option<BoardShape>, after: Option<BoardShape>) -> &mut Self {
        if before != after {
            self.shapes.push((before, after));
        }
        self
    }

    pub fn link(&mut self, before: Option<BoardLink>, after: Option<BoardLink>) -> &mut Self {
        if before != after {
            self.links.push((before, after));
        }
        self
    }

    /// What the change leaves: the shapes and links to save, and the ids to
    /// take off the board.
    pub fn outcome(&self) -> Outcome {
        let mut out = Outcome::default();
        for (before, after) in &self.shapes {
            match (before, after) {
                (_, Some(s)) => out.save_shapes.push(s.clone()),
                (Some(s), None) => out.remove_shapes.push(s.id.clone()),
                (None, None) => {}
            }
        }
        for (before, after) in &self.links {
            match (before, after) {
                (_, Some(l)) => out.save_links.push(l.clone()),
                (Some(l), None) => out.remove_links.push(l.id.clone()),
                (None, None) => {}
            }
        }
        out
    }
}

/// What applying a [`Change`] does to the board.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub save_shapes: Vec<BoardShape>,
    pub remove_shapes: Vec<String>,
    pub save_links: Vec<BoardLink>,
    pub remove_links: Vec<String>,
}

/// One step back or forward: the ink to go to (when the step changed the
/// ink), and the shapes and links to change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub ink: Option<Vec<InkStroke>>,
    pub change: Change,
}

/// The history, up to [`HISTORY`] steps back.
#[derive(Debug, Clone, Default)]
pub struct BoardHistory {
    past: Vec<Step>,
    future: Vec<Step>,
}

impl BoardHistory {
    /// A change that can be taken back: the ink as it was (when the change
    /// touched the ink) and the shapes and links it changed. The redo list goes.
    pub fn commit(&mut self, ink_before: Option<Vec<InkStroke>>, change: Change) {
        if ink_before.is_none() && change.is_empty() {
            return;
        }
        self.past.push(Step {
            ink: ink_before,
            change,
        });
        if self.past.len() > HISTORY {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// ⌘Z from `ink` as it stands: the step to apply (its ink to set, its
    /// change already turned back), if there's anything to undo.
    pub fn undo(&mut self, ink: &[InkStroke]) -> Option<Step> {
        let step = self.past.pop()?;
        self.future.push(Step {
            ink: step.ink.as_ref().map(|_| ink.to_vec()),
            change: step.change.clone(),
        });
        Some(Step {
            ink: step.ink,
            change: step.change.reversed(),
        })
    }

    /// ⇧⌘Z: the step undone last, again. Redo pushes onto the undo list
    /// without trimming it, as 0.9.4's ink did.
    pub fn redo(&mut self, ink: &[InkStroke]) -> Option<Step> {
        let step = self.future.pop()?;
        self.past.push(Step {
            ink: step.ink.as_ref().map(|_| ink.to_vec()),
            change: step.change.clone(),
        });
        Some(step)
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_domain::{InkColor, InkKind, ShapeKind};
    use serde_json::Map;

    fn shape(id: &str, x: f64) -> BoardShape {
        BoardShape {
            id: id.into(),
            kind: ShapeKind::Rect,
            x,
            y: 0.0,
            w: 100.0,
            h: 60.0,
            color: InkColor::Blue,
            text: String::new(),
            image: None,
            by: None,
            created_at: 1.0,
            extra: Map::new(),
        }
    }

    fn pen(id: &str) -> InkStroke {
        InkStroke {
            id: id.into(),
            kind: InkKind::Pen,
            color: InkColor::Red,
            points: vec![0.0, 0.0],
            size: 1.0,
            text: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn undoes_and_redoes_shapes_one_by_one() {
        let mut history = BoardHistory::default();
        let mut add = Change::default();
        add.shape(None, Some(shape("a", 0.0)));
        history.commit(None, add);
        let mut moved = Change::default();
        moved.shape(Some(shape("a", 0.0)), Some(shape("a", 50.0)));
        history.commit(None, moved);

        let back = history.undo(&[]).unwrap();
        assert_eq!(back.ink, None);
        assert_eq!(back.change.outcome().save_shapes, vec![shape("a", 0.0)]);
        let back = history.undo(&[]).unwrap();
        assert_eq!(back.change.outcome().remove_shapes, vec!["a".to_owned()]);
        assert!(history.undo(&[]).is_none());

        let again = history.redo(&[]).unwrap();
        assert_eq!(again.change.outcome().save_shapes, vec![shape("a", 0.0)]);
        let again = history.redo(&[]).unwrap();
        assert_eq!(again.change.outcome().save_shapes, vec![shape("a", 50.0)]);
        assert!(history.redo(&[]).is_none());
    }

    #[test]
    fn takes_ink_back_as_whole_lists_beside_shapes() {
        let mut history = BoardHistory::default();
        let mut erased = Change::default();
        erased.shape(Some(shape("a", 0.0)), None);
        // The eraser took a stroke and a shape in one go.
        history.commit(Some(vec![pen("p")]), erased);
        let back = history.undo(&[]).unwrap();
        assert_eq!(back.ink, Some(vec![pen("p")]));
        assert_eq!(back.change.outcome().save_shapes, vec![shape("a", 0.0)]);
        let again = history.redo(&[pen("p")]).unwrap();
        assert_eq!(again.ink, Some(Vec::new()));
        assert_eq!(again.change.outcome().remove_shapes, vec!["a".to_owned()]);
    }

    #[test]
    fn a_change_that_changed_nothing_is_not_kept() {
        let mut history = BoardHistory::default();
        let mut same = Change::default();
        same.shape(Some(shape("a", 0.0)), Some(shape("a", 0.0)));
        history.commit(None, same);
        assert!(!history.can_undo());
    }
}
