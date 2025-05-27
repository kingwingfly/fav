mod api;
mod command;
mod db;
mod entity;
mod event;
mod migration;
mod payload;
mod response;
mod runtime;
mod system;
mod version;
mod wbi;

use bevy_ecs::{
    schedule::{ExecutorKind, Schedule},
    world::World,
};
use command::FavCommand;

fn main() {
    let event = FavCommand::new().run();

    let mut world = World::new();
    let runtime = runtime::Runtime::new();
    world.insert_resource(runtime.block_on(db::Db::connect()));
    world.insert_resource(runtime);

    let mut schedule = Schedule::default();
    schedule.set_executor_kind(ExecutorKind::SingleThreaded);

    schedule.add_systems(system::auth);
    schedule.run(&mut world);

    world.trigger(event);
}
