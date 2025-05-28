use bevy_ecs::event::Event;

#[derive(Debug, Event)]
pub struct Login;

#[derive(Debug, Event)]
pub struct Logout {
    pub user_id: i32,
}

#[derive(Debug, Event)]
pub struct ListUser;
