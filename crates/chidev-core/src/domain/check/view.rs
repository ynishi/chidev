//! What a check reads: the model before and after the change, and what is kept beside the Project.

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;

/// The model a check is run on. It is everything a check may look at, so that a check is a function of it alone.
#[derive(Debug, Clone, Copy)]
pub struct CheckView<'a> {
    /// The Project before the change; none when nothing changed (the on-demand hook) or none was kept.
    pub before: Option<&'a Project>,
    /// The Project as it is, or as the use case would keep it.
    pub after: &'a Project,
    /// Every Pin, as the use case would leave them.
    pub pins: &'a [Pin],
    /// Every change request, as the use case would leave them.
    pub change_requests: &'a [ChangeRequest],
}
