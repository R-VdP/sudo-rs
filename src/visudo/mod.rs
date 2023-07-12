mod cli;
mod help;

use std::{
    ffi::{CStr, CString, OsString},
    fs::{File, Permissions},
    io::{self, Read, Seek, Write},
    os::unix::prelude::{MetadataExt, OsStringExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    sudoers::Sudoers,
    system::{
        file::{Chown, Lockable},
        User,
    },
};

use self::cli::{VisudoAction, VisudoOptions};
use self::help::{long_help_message, USAGE_MSG};

const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! io_msg {
    ($err:expr, $($tt:tt)*) => {
        io::Error::new($err.kind(), format!("{}: {}", format_args!($($tt)*), $err))
    };
}

pub fn main() {
    let options = match VisudoOptions::from_env() {
        Ok(options) => options,
        Err(error) => {
            println!("visudo: {error}\n{USAGE_MSG}");
            std::process::exit(1);
        }
    };

    let cmd = match options.action {
        VisudoAction::Help => {
            println!("{}", long_help_message());
            std::process::exit(0);
        }
        VisudoAction::Version => {
            println!("visudo version {VERSION}");
            std::process::exit(0);
        }
        VisudoAction::Check => check,
        VisudoAction::Run => run,
    };

    match cmd(options.file.as_deref(), options.perms, options.owner) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("visudo: {error}");
            std::process::exit(1);
        }
    }
}

fn check(file_arg: Option<&str>, perms: bool, owner: bool) -> io::Result<()> {
    let sudoers_path = Path::new(file_arg.unwrap_or("/etc/sudoers"));

    let sudoers_file = File::open(sudoers_path)
        .map_err(|err| io_msg!(err, "unable to open {}", sudoers_path.display()))?;

    let metadata = sudoers_file.metadata()?;

    if file_arg.is_none() || perms {
        // For some reason, the MSB of the mode is on so we need to mask it.
        let mode = metadata.permissions().mode() & 0o777;

        if mode != 0o440 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "{}: bad permissions, should be mode 0440, but found {mode:04o}",
                    sudoers_path.display()
                ),
            ));
        }
    }

    if file_arg.is_none() || owner {
        let owner = (metadata.uid(), metadata.gid());

        if owner != (0, 0) {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "{}: wrong owner (uid, gid) should be (0, 0), but found {owner:?}",
                    sudoers_path.display()
                ),
            ));
        }
    }

    let (_sudoers, errors) = Sudoers::read(&sudoers_file)?;

    if errors.is_empty() {
        println!("{}: parsed OK", sudoers_path.display());
        return Ok(());
    }

    for crate::sudoers::Error(_position, message) in errors {
        eprintln!("syntax error: {message}");
    }

    Err(io::Error::new(io::ErrorKind::Other, "invalid sudoers file"))
}

fn run(file_arg: Option<&str>, perms: bool, owner: bool) -> io::Result<()> {
    let sudoers_path = Path::new(file_arg.unwrap_or("/etc/sudoers"));

    let (mut sudoers_file, existed) = if sudoers_path.exists() {
        let file = File::options().read(true).write(true).open(sudoers_path)?;

        (file, true)
    } else {
        // Create a sudoers file if it doesn't exist.
        let file = File::create(sudoers_path)?;
        // ogvisudo sets the permissions of the file so it can be read and written by the user and
        // read by the group if the `-f` argument was passed.
        if file_arg.is_some() {
            file.set_permissions(Permissions::from_mode(0o640))?;
        }
        (file, false)
    };

    sudoers_file.lock_exclusive(true).map_err(|err| {
        if err.kind() == io::ErrorKind::WouldBlock {
            io_msg!(err, "{} busy, try again later", sudoers_path.display())
        } else {
            err
        }
    })?;

    let result: io::Result<()> = (|| {
        if perms || file_arg.is_none() {
            sudoers_file.set_permissions(Permissions::from_mode(0o440))?;
        }

        if owner || file_arg.is_none() {
            sudoers_file.chown(User::real_uid(), User::real_gid())?;
        }

        let tmp_path = create_temporary_dir()?.join("sudoers");

        let mut tmp_file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .open(&tmp_path)?;
        tmp_file.set_permissions(Permissions::from_mode(0o700))?;

        let mut sudoers_contents = Vec::new();
        if existed {
            // If the sudoers file existed, read its contents and write them into the temporary file.
            sudoers_file.read_to_end(&mut sudoers_contents)?;
            // Rewind the sudoers file so it can be written later.
            sudoers_file.rewind()?;
            // Write to the temporary file.
            tmp_file.write_all(&sudoers_contents)?;
        }

        let editor_path = solve_editor_path()?;

        loop {
            Command::new(&editor_path)
                .arg("--")
                .arg(&tmp_path)
                .spawn()?
                .wait_with_output()?;

            let (_sudoers, errors) = File::open(&tmp_path)
                .and_then(|reader| Sudoers::read(reader, &tmp_path))
                .map_err(|err| {
                    io_msg!(
                        err,
                        "unable to re-open temporary file ({}), {} unchanged",
                        tmp_path.display(),
                        sudoers_path.display()
                    )
                })?;

            if errors.is_empty() {
                break;
            }

            eprintln!("Come on... you can do better than that.\n");

            for crate::sudoers::Error(_position, message) in errors {
                eprintln!("syntax error: {message}");
            }

            eprintln!();

            let stdin = io::stdin();
            let stdout = io::stdout();

            let mut stdin_handle = stdin.lock();
            let mut stdout_handle = stdout.lock();

            loop {
                stdout_handle
                    .write_all("What now? e(x)it without saving / (e)dit again: ".as_bytes())?;
                stdout_handle.flush()?;

                let mut input = [0u8];
                if let Err(err) = stdin_handle.read_exact(&mut input) {
                    eprintln!("visudo: cannot read user input: {err}");
                    return Ok(());
                }

                match &input {
                    b"e" => break,
                    b"x" => return Ok(()),
                    input => println!("Invalid option: {:?}\n", std::str::from_utf8(input)),
                }
            }
        }

        let tmp_contents = std::fs::read(&tmp_path)?;
        // Only write to the sudoers file if the contents changed.
        if tmp_contents == sudoers_contents {
            eprintln!("visudo: {} unchanged", tmp_path.display());
        } else {
            sudoers_file.write_all(&tmp_contents)?;
        }

        Ok(())
    })();

    sudoers_file.unlock()?;

    result?;

    Ok(())
}

fn solve_editor_path() -> io::Result<PathBuf> {
    let path = Path::new("/usr/bin/editor");
    if path.exists() {
        return Ok(path.to_owned());
    }

    for key in ["SUDO_EDITOR", "VISUAL", "EDITOR"] {
        if let Some(var) = std::env::var_os(key) {
            let path = Path::new(&var);
            if path.exists() {
                return Ok(path.to_owned());
            }
        }
    }

    let path = Path::new("/usr/bin/vi");
    if path.exists() {
        return Ok(path.to_owned());
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "cannot find text editor",
    ))
}

macro_rules! cstr {
    ($expr:expr) => {{
        let _: &'static [u8] = $expr;
        debug_assert!(std::ffi::CStr::from_bytes_with_nul($expr).is_ok());
        // SAFETY: see `debug_assert!` above
        unsafe { CStr::from_bytes_with_nul_unchecked($expr) }
    }};
}

fn create_temporary_dir() -> io::Result<PathBuf> {
    let template = cstr!(b"/tmp/sudoers-XXXXXX\0").to_owned();

    let ptr = unsafe { libc::mkdtemp(template.into_raw()) };

    if ptr.is_null() {
        return Err(io::Error::last_os_error());
    }

    let path = OsString::from_vec(unsafe { CString::from_raw(ptr) }.into_bytes()).into();

    Ok(path)
}
