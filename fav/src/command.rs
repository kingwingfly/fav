use anyhow::Result;
use bevy_ecs::{
    schedule::{ExecutorKind, Schedule, ScheduleLabel},
    world::World,
};
use clap::{Arg, ArgAction, Command, command, value_parser};

use crate::{
    db::Db,
    event::{ListUser, Login, Logout, LogoutAll},
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
                            .args([
                                Arg::new("all")
                                    .help("Logout all authorized users")
                                    .long("all")
                                    .short('a')
                                    .action(ArgAction::SetTrue)
                                    .conflicts_with("user_id"),
                                Arg::new("user_id")
                                    .help("The user to logout")
                                    .value_parser(value_parser!(i32))
                                    .action(ArgAction::Append),
                            ]),
                        Command::new("list").about("List all authorized users"),
                    ])]),
        )
    }

    /// Parse the commands and args, return the Event to trigger.
    pub fn run(self) -> Result<()> {
        let matches = self.0.get_matches();

        let mut world = World::new();
        let mut schedule = Schedule::new(FavSchedule);
        schedule.set_executor_kind(ExecutorKind::SingleThreaded);

        let runtime = Runtime::new()?;
        let db = runtime.block_on(Db::connect())?;
        world.insert_resource(db);
        world.insert_resource(runtime);

        schedule.add_systems(system::auth);
        world.add_schedule(schedule);

        world.run_schedule(FavSchedule);

        match matches.subcommand() {
            Some(("auth", sub_matches)) => match sub_matches.subcommand() {
                Some(("login", _)) => world.trigger(Login),
                Some(("logout", sub_matches)) if sub_matches.get_flag("all") => {
                    world.trigger(LogoutAll);
                    // run again for events triggered by events
                    world.run_schedule(FavSchedule);
                }
                Some(("logout", sub_matches)) => sub_matches
                    .get_many::<i32>("user_id")
                    .unwrap() // required has been set to true
                    .for_each(|&user_id| {
                        world.trigger(Logout { user_id });
                    }),
                Some(("list", _)) => world.trigger(ListUser),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, ScheduleLabel)]
struct FavSchedule;
