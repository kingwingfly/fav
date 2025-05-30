use anyhow::Result;
use bevy_ecs::{
    schedule::{ExecutorKind, Schedule, ScheduleLabel},
    world::World,
};
use clap::{Arg, ArgAction, Command, command, value_parser};
use clap_complete::Shell;
use tracing_subscriber::EnvFilter;

use crate::{
    db::Db,
    event::{
        ActivateAccount, ActivateAccountAll, ActivateSet, ActivateSetAll, ActivateUp,
        ActivateUpAll, DeactivateAccount, DeactivateAccountAll, DeactivateSet, DeactivateSetAll,
        DeactivateUp, DeactivateUpAll, Fetch, ListAccount, ListMedia, ListSet, ListUp, Login,
        Logout, LogoutAll, Pull,
    },
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
                                        .value_parser(value_parser!(i64))
                                        .action(ArgAction::Append),
                                ]),
                        ]),
                    Command::new("list")
                        .about("List accounts/sets/medias [alias: ls, l]")
                        .arg_required_else_help(true)
                        .aliases(["ls", "l"])
                        .subcommands([
                            Command::new("account")
                                .about("List accounts [alias: a]")
                                .aliases(["a"]),
                            Command::new("set")
                                .about("List sets [alias: list, collection, s, l, c]")
                                .aliases(["list", "collection", "s", "l", "c"]),
                            Command::new("up")
                                .about("List uppers [alias: upper, u]")
                                .aliases(["upper", "u"]),
                            Command::new("media")
                                .about("List medias [alias: video bv, m, v]")
                                .aliases(["video", "bv", "m", "v"]),
                        ]),
                    Command::new("activate")
                        .about("Activate obj [alias: active, a]")
                        .arg_required_else_help(true)
                        .aliases(["active", "a"])
                        .subcommands([
                            Command::new("account")
                                .about("Activate accounts [alias: a]")
                                .arg_required_else_help(true)
                                .aliases(["a"])
                                .args([
                                    Arg::new("all")
                                        .help("Activate all authorized accounts")
                                        .long("all")
                                        .short('a')
                                        .action(ArgAction::SetTrue)
                                        .conflicts_with("account_id"),
                                    Arg::new("account_id")
                                        .help("The account to activate")
                                        .value_parser(value_parser!(i64))
                                        .action(ArgAction::Append),
                                ]),
                            Command::new("set")
                                .about("Activate sets [alias: list, collection, s, l, c]")
                                .arg_required_else_help(true)
                                .aliases(["list", "collection", "s", "l", "c"])
                                .args([
                                    Arg::new("all")
                                        .help("Activate all sets")
                                        .long("all")
                                        .short('a')
                                        .action(ArgAction::SetTrue)
                                        .conflicts_with("set_id"),
                                    Arg::new("set_id")
                                        .help("The set to activate")
                                        .value_parser(value_parser!(i64))
                                        .action(ArgAction::Append),
                                ]),
                            Command::new("up")
                                .about("Activate ups [alias: u]")
                                .arg_required_else_help(true)
                                .aliases(["u"])
                                .args([
                                    Arg::new("all")
                                        .help("Activate all ups")
                                        .long("all")
                                        .short('a')
                                        .action(ArgAction::SetTrue)
                                        .conflicts_with("up_id"),
                                    Arg::new("up_id")
                                        .help("The up to activate")
                                        .value_parser(value_parser!(i64))
                                        .action(ArgAction::Append),
                                ]),
                        ]),
                    Command::new("deactivate")
                        .about("Deactivate authorized accounts [alias: d]")
                        .aliases(["d"])
                        .arg_required_else_help(true)
                        .subcommands([
                            Command::new("account")
                                .about("Deactivate accounts [alias: a]")
                                .arg_required_else_help(true)
                                .aliases(["a"])
                                .args([
                                    Arg::new("all")
                                        .help("Dectivate all authorized accounts")
                                        .long("all")
                                        .short('a')
                                        .action(ArgAction::SetTrue)
                                        .conflicts_with("account_id"),
                                    Arg::new("account_id")
                                        .help("The account to deactivate")
                                        .value_parser(value_parser!(i64))
                                        .action(ArgAction::Append),
                                ]),
                            Command::new("set")
                                .about("Deactivate sets [alias: list, s, l]")
                                .arg_required_else_help(true)
                                .aliases(["list", "s", "l"])
                                .args([
                                    Arg::new("all")
                                        .help("Deactivate all sets")
                                        .long("all")
                                        .short('a')
                                        .action(ArgAction::SetTrue)
                                        .conflicts_with("set_id"),
                                    Arg::new("set_id")
                                        .help("The set to deactivate")
                                        .value_parser(value_parser!(i64))
                                        .action(ArgAction::Append),
                                ]),
                            Command::new("up")
                                .about("Deactivate ups [alias: u]")
                                .arg_required_else_help(true)
                                .aliases(["u"])
                                .args([
                                    Arg::new("all")
                                        .help("Deactivate all ups")
                                        .long("all")
                                        .short('a')
                                        .action(ArgAction::SetTrue)
                                        .conflicts_with("up_id"),
                                    Arg::new("up_id")
                                        .help("The up to deactivate")
                                        .value_parser(value_parser!(i64))
                                        .action(ArgAction::Append),
                                ]),
                        ]),
                    Command::new("fetch")
                        .about("Fetch metadata of following ups, fav sets, medias, ups [alias: f]")
                        .aliases(["f"]),
                    Command::new("pull")
                        .about("Pull fetched medias [alias: p]")
                        .aliases(["p"]),
                    Command::new("completion")
                        .about("Generate completion script")
                        .arg_required_else_help(true)
                        .args([Arg::new("shell")
                            .help("The shell to generate completion script for")
                            .value_parser(value_parser!(Shell))]),
                ])
                .args([Arg::new("verbose")
                    .help("Show debug messages")
                    .long("verbose")
                    .short('v')
                    .action(ArgAction::SetTrue)]),
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
                let mut log_level = "fav=info";
                if matches.get_flag("verbose") {
                    log_level = "fav=debug";
                }
                let filter =
                    EnvFilter::from_default_env().add_directive(log_level.parse().unwrap());
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_writer(std::io::stdout)
                    .with_line_number(true)
                    .with_thread_ids(true)
                    .init();
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
                    system::fetch,
                    system::pull,
                ));
                world.add_schedule(schedule);

                world.run_schedule(FavSchedule);

                match sub_cmd {
                    Some(("auth", sub_matches)) => match sub_matches.subcommand() {
                        Some(("login", _)) => world.trigger(Login),
                        Some(("logout", sub_matches)) if sub_matches.get_flag("all") => {
                            world.trigger(LogoutAll)
                        }
                        Some(("logout", sub_matches)) => sub_matches
                            .get_many::<i64>("account_id")
                            .unwrap() // arg_required_else_help has been set to true
                            .for_each(|&account_id| {
                                world.trigger(Logout { account_id });
                            }),

                        _ => unreachable!(),
                    },
                    Some(("list", sub_matches)) => match sub_matches.subcommand() {
                        Some(("account", _)) => world.trigger(ListAccount),
                        Some(("set", _)) => world.trigger(ListSet),
                        Some(("up", _)) => world.trigger(ListUp),
                        Some(("media", _)) => world.trigger(ListMedia),
                        _ => unreachable!(),
                    },
                    Some(("activate", sub_matches)) => match sub_matches.subcommand() {
                        Some(("account", sub_matches)) => match sub_matches.get_flag("all") {
                            true => world.trigger(ActivateAccountAll),
                            false => sub_matches
                                .get_many::<i64>("account_id")
                                .unwrap() // arg_required_else_help has been set to true
                                .for_each(|&account_id| {
                                    world.trigger(ActivateAccount { account_id });
                                }),
                        },
                        Some(("set", sub_matches)) => match sub_matches.get_flag("all") {
                            true => world.trigger(ActivateSetAll),
                            false => sub_matches
                                .get_many::<i64>("set_id")
                                .unwrap() // arg_required_else_help has been set to true
                                .for_each(|&set_id| {
                                    world.trigger(ActivateSet { set_id });
                                }),
                        },
                        Some(("up", sub_matches)) => match sub_matches.get_flag("all") {
                            true => world.trigger(ActivateUpAll),
                            false => sub_matches
                                .get_many::<i64>("up_id")
                                .unwrap() // arg_required_else_help has been set to true
                                .for_each(|&up_id| {
                                    world.trigger(ActivateUp { up_id });
                                }),
                        },
                        _ => unreachable!(),
                    },
                    Some(("deactivate", sub_matches)) => match sub_matches.subcommand() {
                        Some(("account", sub_matches)) => match sub_matches.get_flag("all") {
                            true => world.trigger(DeactivateAccountAll),
                            false => sub_matches
                                .get_many::<i64>("account_id")
                                .unwrap() // arg_required_else_help has been set to true
                                .for_each(|&account_id| {
                                    world.trigger(DeactivateAccount { account_id });
                                }),
                        },
                        Some(("set", sub_matches)) => match sub_matches.get_flag("all") {
                            true => world.trigger(DeactivateSetAll),
                            false => sub_matches
                                .get_many::<i64>("set_id")
                                .unwrap() // arg_required_else_help has been set to true
                                .for_each(|&set_id| {
                                    world.trigger(DeactivateSet { set_id });
                                }),
                        },
                        Some(("up", sub_matches)) => match sub_matches.get_flag("all") {
                            true => world.trigger(DeactivateUpAll),
                            false => sub_matches
                                .get_many::<i64>("up_id")
                                .unwrap() // arg_required_else_help has been set to true
                                .for_each(|&up_id| {
                                    world.trigger(DeactivateUp { up_id });
                                }),
                        },
                        _ => unreachable!(),
                    },
                    Some(("fetch", _)) => world.trigger(Fetch),
                    Some(("pull", _)) => world.trigger(Pull),
                    _ => unreachable!(),
                }

                world.run_schedule(FavSchedule);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, ScheduleLabel)]
struct FavSchedule;
