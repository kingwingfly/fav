use bevy_ecs::event::Event;

#[derive(Debug, Event)]
pub struct Login;

#[derive(Debug, Event)]
pub struct Logout {
    pub user_id: i32,
}

#[derive(Debug, Event)]
pub struct LogoutAll;

#[derive(Debug, Event)]
pub struct ListUser;

#[derive(Debug, Event)]
pub struct Activate {
    pub user_id: i32,
}

#[derive(Debug, Event)]
pub struct ActivateAll;

#[derive(Debug, Event)]
pub struct Deactivate {
    pub user_id: i32,
}

#[derive(Debug, Event)]
pub struct DeactivateAll;
