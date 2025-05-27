use bevy_ecs::event::Event;
use clap::{Command, command};

use crate::{event::Login, version::VERSION};

#[derive(Debug, Default)]
pub struct FavCommand(Command);

impl FavCommand {
    pub fn new() -> Self {
        Self(
            command!()
                .version(VERSION)
                .subcommands([Command::new("auth")
                    .about("Auth your account")
                    .subcommands([
                        Command::new("login").about("Login with QR code"),
                        Command::new("logout").about("Logout"),
                    ])]),
        )
    }

    /// Parse the commands and args, return the Event to trigger.
    pub fn run(self) -> impl Event {
        let matches = self.0.get_matches();
        match matches.subcommand() {
            Some(("auth", sub_matches)) => match sub_matches.subcommand() {
                Some(("login", _)) => Login,
                Some(("logout", _)) => todo!(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }
}
