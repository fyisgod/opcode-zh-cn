pub mod registry;

pub use registry::*;

use std::{ffi::OsStr, process::Command};

/// Creates a non-interactive command without opening a console on Windows.
pub(crate) fn background_command(program: impl AsRef<OsStr>) -> Command {
    let cmd = Command::new(program);

    #[cfg(windows)]
    let cmd = {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = cmd;
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    };

    cmd
}
