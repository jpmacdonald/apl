//! Completions command

use clap::CommandFactory;
use clap_complete::generate;

/// Generate shell completions
pub fn completions(shell: clap_complete::Shell) {
    let mut cmd = crate::Cli::command();
    generate(shell, &mut cmd, "dl", &mut std::io::stdout());
}
