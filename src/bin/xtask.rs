use {
    anyhow::Result,
    clap::{Args, Parser, Subcommand},
    log::error,
};

#[derive(Parser)]
#[command(name = "xtask", about = "Build tasks", version)]
struct Xtask {
    #[command(flatten)]
    pub global: GlobalOptions,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Hello")]
    Hello,
    #[command(about = "Bump version")]
    BumpVersion(xtask::commands::bump_version::CommandArgs),
    #[command(about = "Update crate version")]
    UpdateCrate(xtask::commands::update_crate::CommandArgs),
    #[command(about = "Publish crates")]
    Publish(xtask::commands::publish::CommandArgs),
}

#[derive(Args, Debug)]
pub struct GlobalOptions {
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[tokio::main]
async fn main() {
    if let Err(err) = try_main().await {
        error!("Error: {err}");
        for (i, cause) in err.chain().skip(1).enumerate() {
            error!("  {}: {}", i.saturating_add(1), cause);
        }
        std::process::exit(1);
    }
}

async fn try_main() -> Result<()> {
    let xtask = Xtask::parse();

    if xtask.global.verbose {
        std::env::set_var("RUST_LOG", "debug");
    } else {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::init();

    match xtask.command {
        Commands::Hello => xtask::commands::hello::run()?,
        Commands::BumpVersion(args) => {
            xtask::commands::bump_version::run(args)?;
        }
        Commands::UpdateCrate(args) => {
            xtask::commands::update_crate::run(args)?;
        }
        Commands::Publish(args) => {
            xtask::commands::publish::run(args)?;
        }
    }

    Ok(())
}
