use anyhow::Result;
use bevy_ecs::{
    schedule::{ExecutorKind, Schedule, ScheduleLabel},
    world::World,
};
use clap::{Arg, ArgAction, Command, command, value_parser};
use clap_complete::Shell;

use crate::{
    db::Db,
    event::{Activate, ActivateAll, Deactivate, DeactivateAll, ListUser, Login, Logout, LogoutAll},
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
                .subcommands([
                    Command::new("auth")
                        .about("Auth account")
                        .arg_required_else_help(true)
                        .subcommands([
                            Command::new("login").about("Login with QR code"),
                            Command::new("logout")
                                .about("Logout")
                                .arg_required_else_help(true)
                                .args([
                                    Arg::new("all")
                                        .help("Logout all authorized accounts")
                                        .long("all")
                                        .short('a')
                                        .action(ArgAction::SetTrue)
                                        .conflicts_with("account_id"),
                                    Arg::new("account_id")
                                        .help("The account to logout")
                                        .value_parser(value_parser!(i32))
                                        .action(ArgAction::Append),
                                ]),
                        ]),
                    Command::new("list")
                        .about("List accounts/sets/videos [alias: ls, l]")
                        .arg_required_else_help(true)
                        .aliases(["ls", "l"])
                        .subcommands([
                            Command::new("account")
                                .about("List accounts [alias: user, a , u]")
                                .aliases(["user", "a", "u"]),
                            Command::new("set")
                                .about("List sets [alias: list, s, l]")
                                .aliases(["list", "s", "l"]),
                            Command::new("up")
                                .about("List uppers [alias: upper]")
                                .aliases(["upper"]),
                            Command::new("video")
                                .about("List videos [alias: bv, v]")
                                .aliases(["bv", "v"]),
                        ]),
                    Command::new("activate")
                        .about("Activate authorized accounts")
                        .arg_required_else_help(true)
                        .args([
                            Arg::new("all")
                                .help("Activate all authorized accounts")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("account_id"),
                            Arg::new("account_id")
                                .help("The account to activate")
                                .value_parser(value_parser!(i32))
                                .action(ArgAction::Append),
                        ]),
                    Command::new("deactivate")
                        .about("Deactivate authorized accounts")
                        .arg_required_else_help(true)
                        .args([
                            Arg::new("all")
                                .help("Deactivate all authorized accounts")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("account_id"),
                            Arg::new("account_id")
                                .help("The account to deactivate")
                                .value_parser(value_parser!(i32))
                                .action(ArgAction::Append),
                        ]),
                    Command::new("completion")
                        .about("Generate completion script")
                        .arg_required_else_help(true)
                        .args([Arg::new("shell")
                            .help("The shell to generate completion script for")
                            .value_parser(value_parser!(Shell))]),
                ]),
        )
    }

    /// Parse the commands and args, return the Event to trigger.
    pub fn run(mut self) -> Result<()> {
        let matches = self.0.get_matches_mut();

        match matches.subcommand() {
            Some(("completion", sub_matches)) => {
                let bin_name = self.0.get_name().to_string();
                let shell = *sub_matches.get_one::<Shell>("shell").unwrap();
                clap_complete::generate(shell, &mut self.0, bin_name, &mut std::io::stdout());
            }
            sub_cmd => {
                let mut world = World::new();
                let mut schedule = Schedule::new(FavSchedule);
                schedule.set_executor_kind(ExecutorKind::SingleThreaded);

                let runtime = Runtime::new()?;
                let db = runtime.block_on(Db::connect())?;
                world.insert_resource(db);
                world.insert_resource(runtime);

                schedule.add_systems((
                    system::auth,
                    system::activate,
                    system::deactivate,
                    system::list,
                ));
                world.add_schedule(schedule);

                world.run_schedule(FavSchedule);

                match sub_cmd {
                    Some(("auth", sub_matches)) => match sub_matches.subcommand() {
                        Some(("login", _)) => world.trigger(Login),
                        Some(("logout", sub_matches)) if sub_matches.get_flag("all") => {
                            world.trigger(LogoutAll);
                            // run again for events triggered by events
                            world.run_schedule(FavSchedule);
                        }
                        Some(("logout", sub_matches)) => sub_matches
                            .get_many::<i32>("account_id")
                            .unwrap() // arg_required_else_help has been set to true
                            .for_each(|&account_id| {
                                world.trigger(Logout { account_id });
                            }),
                        _ => unreachable!(),
                    },
                    Some(("list", sub_matches)) => match sub_matches.subcommand() {
                        Some(("account", _)) => world.trigger(ListUser),
                        Some(("set", _)) => todo!(),
                        Some(("up", _)) => todo!(),
                        Some(("video", _)) => todo!(),
                        _ => unreachable!(),
                    },
                    Some(("activate", sub_matches)) => match sub_matches.get_flag("all") {
                        true => {
                            world.trigger(ActivateAll);
                            // run again for events triggered by events
                            world.run_schedule(FavSchedule);
                        }
                        false => sub_matches
                            .get_many::<i32>("account_id")
                            .unwrap() // arg_required_else_help has been set to true
                            .for_each(|&account_id| {
                                world.trigger(Activate { account_id });
                            }),
                    },
                    Some(("deactivate", sub_matches)) => match sub_matches.get_flag("all") {
                        true => {
                            world.trigger(DeactivateAll);
                            // run again for events triggered by events
                            world.run_schedule(FavSchedule);
                        }
                        false => sub_matches
                            .get_many::<i32>("account_id")
                            .unwrap() // arg_required_else_help has been set to true
                            .for_each(|&account_id| {
                                world.trigger(Deactivate { account_id });
                            }),
                    },
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, ScheduleLabel)]
struct FavSchedule;
