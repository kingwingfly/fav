use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match fav_bili::command::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            err.exit_code()
        }
    }
}
