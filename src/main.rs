use std::error::Error;

use parse::parse;
use run::run;

mod deserialize;
mod parse;
mod run;
mod native;

fn main() -> Result<(), Box<dyn Error>> {
    let cli = clap::Command::new("matebabe")
        .subcommand_required(true)
        .subcommand(clap::Command::new("parse").arg(clap::arg!(<FILE> "file to parse")))
        .subcommand(clap::Command::new("run").arg(clap::arg!(<FILE> "file to run")));

    let matches = cli.get_matches();
    match matches.subcommand() {
        Some(("parse", submatches)) => {
            let deserialized = deserialize::deserialize_class_file(
                submatches
                    .get_one::<String>("FILE")
                    .expect("required")
                    .to_string(),
            )?;

            parse(deserialized)?;
        }
        Some(("run", submatches)) => {
            let filename = submatches
                .get_one::<String>("FILE")
                .expect("required")
                .to_string();
            run(filename)
        }
        Some(_) => println!("Command not found :("),
        None => println!("Command not found :("),
    }
    Ok(())
}
