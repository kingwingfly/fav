use bevy_ecs::event::Event;

#[derive(Debug, Event)]
pub struct Login;

#[derive(Debug, Event)]
pub struct Logout {
    pub account_id: i64,
}

#[derive(Debug, Event)]
pub struct LogoutAll;

#[derive(Debug, Event)]
pub struct ListAccount;

#[derive(Debug, Event)]
pub struct ListSet;

#[derive(Debug, Event)]
pub struct ListUp;

#[derive(Debug, Event)]
pub struct ListMedia;

#[derive(Debug, Event)]
pub struct ActivateAccount {
    pub account_id: i64,
}

#[derive(Debug, Event)]
pub struct ActivateAccountAll;

#[derive(Debug, Event)]
pub struct DeactivateAccount {
    pub account_id: i64,
}

#[derive(Debug, Event)]
pub struct DeactivateAccountAll;

#[derive(Debug, Event)]
pub struct ActivateSet {
    pub set_id: i64,
}

#[derive(Debug, Event)]
pub struct ActivateSetAll;

#[derive(Debug, Event)]
pub struct DeactivateSet {
    pub set_id: i64,
}

#[derive(Debug, Event)]
pub struct DeactivateSetAll;

#[derive(Debug, Event)]
pub struct ActivateUp {
    pub up_id: i64,
}

#[derive(Debug, Event)]
pub struct ActivateUpAll;

#[derive(Debug, Event)]
pub struct DeactivateUp {
    pub up_id: i64,
}

#[derive(Debug, Event)]
pub struct DeactivateUpAll;

#[derive(Debug, Event)]
pub struct Fetch {
    pub prune: bool,
}

#[derive(Debug, Event)]
pub struct Pull;
