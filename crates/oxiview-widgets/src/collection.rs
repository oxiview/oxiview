//! Retained collection Views, virtual geometry, and keyed table selection.

mod list;
mod mount;
mod order;
mod table;

pub use list::{ListAnchor, ListRow, VirtualList};
pub use table::{CellKey, CellSelection, RowOrderChange, TableColumn, TableItem, VirtualTable};

use oxiview_core::UiError;
use oxiview_layout::ExtentError;

/// A collection model or retained reconciliation operation failed.
#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    /// A data key appeared more than once on one axis.
    #[error("duplicate virtual collection key {0}")]
    DuplicateKey(u64),
    /// The requested data key is absent from the current order.
    #[error("unknown virtual collection key {0}")]
    UnknownKey(u64),
    /// An axis size or extent is invalid.
    #[error(transparent)]
    Extent(#[from] ExtentError),
    /// An operation on the retained tree failed.
    #[error(transparent)]
    Ui(#[from] UiError),
}

/// Retained root work performed by a successful reconciliation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MountStats {
    /// Total retained items after the reconciliation.
    pub mounted: usize,
    /// Roots created for keys entering the mounted window.
    pub constructed: usize,
    /// Roots released for keys leaving the mounted window.
    pub disposed: usize,
    /// Existing root placements changed by measurement, resizing, or scrolling.
    pub repositioned: usize,
}

/// Body row, data cell, and heading work performed by a table reconciliation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableMountStats {
    /// Body row wrappers.
    pub rows: MountStats,
    /// Data cells, including any application-designated row header column.
    pub cells: MountStats,
    /// Column headings.
    pub headers: MountStats,
}

#[cfg(test)]
mod tests;

impl From<CollectionError> for oxiview_core::Error {
    fn from(error: CollectionError) -> Self {
        match error {
            CollectionError::Ui(error) => Self::Ui(error),
            other => Self::Application(other.to_string()),
        }
    }
}
