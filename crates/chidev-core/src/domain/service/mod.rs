//! Domain services: decisions that read more than one aggregate (a Pin against the Project, the citations of
//! every Case against the assets they cite), so they belong to no single one.

pub mod cite;
pub mod pin;
