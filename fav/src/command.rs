use bevy_ecs::{
    schedule::{ExecutorKind, Schedule},
    world::World,
};
use clap::{Arg, Command, command, value_parser};

use crate::{
    db::Db,
    event::{ListUser, Login, Logout},
    runtime::Runtime,
    system,
    version::VERSION,
};

#[derive(Debug, Default)]
pub struct FavCommand(Command);

impl FavCommand {
    pub fn new() -> Self {
        Self(
            command!()
                .arg_required_else_help(true)
                .version(VERSION)
                .subcommands([Command::new("auth")
                    .about("Auth your account")
                    .arg_required_else_help(true)
                    .subcommands([
                        Command::new("login").about("Login with QR code"),
                        Command::new("logout")
                            .about("Logout")
                            .arg_required_else_help(true)
                            .arg(
                                Arg::new("user_id")
                                    .help("The of user to logout")
                                    .value_parser(value_parser!(i32)),
                            ),
                        Command::new("list").about("List all authorized users"),
                    ])]),
        )
    }

    /// Parse the commands and args, return the Event to trigger.
    pub fn run(self) {
        let matches = self.0.get_matches();

        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.set_executor_kind(ExecutorKind::SingleThreaded);

        let runtime = Runtime::new();
        world.insert_resource(runtime.block_on(Db::connect()));
        world.insert_resource(runtime);

        schedule.add_systems(system::auth);
        schedule.run(&mut world);

        match matches.subcommand() {
            Some(("auth", sub_matches)) => match sub_matches.subcommand() {
                Some(("login", _)) => world.trigger(Login),
                Some(("logout", sub_matches)) => world.trigger(Logout {
                    user_id: *sub_matches.get_one::<i32>("user_id").unwrap(),
                }),
                Some(("list", _)) => world.trigger(ListUser),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }
}
