// src/main.rs — 命令行入口

use clap::Parser;
use riko_memory::commands::{self, Commands};

#[derive(Parser, Debug)]
#[command(
    name = "riko-memory",
    about = "璃的叙事记忆工具（Rust 重写版）",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn main() {
    // 恢复 SIGPIPE 默认行为（SIG_DFL）：
    // Rust 运行时默认忽略 SIGPIPE（SIG_IGN），导致管道下游提前关闭（如 | head）时，
    // println! 等写入 stdout 会遇到 EPIPE 并触发 std::io 的 panic (Broken pipe, exit 101)。
    // 恢复为系统默认处理（SIG_DFL）可让进程在管道中断时由系统内核干净终止（退出码 141），避免 panic 与打印错误堆栈。
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    match cli.command {
        Commands::Add(args) => commands::cmd_add(args),
        Commands::Inbox(args) => commands::cmd_inbox(args),
        Commands::Search(args) => commands::cmd_search(args),
        Commands::Recent(args) => commands::cmd_recent(args),
        Commands::Reveal(args) => commands::cmd_reveal(args),
        Commands::Open => commands::cmd_open(),
        Commands::Resident => commands::cmd_resident(),
        Commands::Summary(args) => commands::cmd_summary(args),
        Commands::Lint => commands::cmd_lint(),
        Commands::SyncAgents => commands::cmd_sync_agents(),
        Commands::Index => commands::cmd_index(),
        Commands::Changelog(args) => commands::cmd_changelog(args),
        Commands::Dream(args) => commands::cmd_dream(args),
        Commands::Facts(args) => commands::cmd_facts(args),
    }
}
