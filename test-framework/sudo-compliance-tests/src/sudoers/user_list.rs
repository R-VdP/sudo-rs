//! Test the first component of the user specification: `<user_list> ALL=(ALL:ALL) ALL`

use pretty_assertions::assert_eq;
use sudo_test::{Command, Env};

use crate::{Result, PAMD_SUDO_PAM_PERMIT, USERNAME};

#[test]
fn no_match() -> Result<()> {
    let env = Env("").build()?;

    let output = Command::new("sudo").arg("true").exec(&env)?;
    assert_eq!(Some(1), output.status().code());

    if sudo_test::is_original_sudo() {
        assert_contains!(output.stderr(), "root is not in the sudoers file");
    }

    Ok(())
}

#[test]
fn all() -> Result<()> {
    let env = Env("ALL ALL=(ALL:ALL) NOPASSWD: ALL")
        .user(USERNAME)
        .build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()?;

    Command::new("sudo")
        .arg("true")
        .as_user(USERNAME)
        .exec(&env)?
        .assert_success()
}

#[test]
fn user_name() -> Result<()> {
    let env = Env("root ALL=(ALL:ALL) NOPASSWD: ALL").build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()
}

#[test]
fn user_id() -> Result<()> {
    let env = Env("#0 ALL=(ALL:ALL) NOPASSWD: ALL").build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()
}

#[test]
fn group_name() -> Result<()> {
    let env = Env("%root ALL=(ALL:ALL) NOPASSWD: ALL").build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()
}

#[test]
fn group_id() -> Result<()> {
    let env = Env("%#0 ALL=(ALL:ALL) NOPASSWD: ALL").build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()
}

#[test]
fn many_different() -> Result<()> {
    let env = Env(format!("root, {USERNAME} ALL=(ALL:ALL) NOPASSWD: ALL"))
        .user(USERNAME)
        .build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()?;

    Command::new("sudo")
        .arg("true")
        .as_user(USERNAME)
        .exec(&env)?
        .assert_success()
}

#[test]
fn many_repeated() -> Result<()> {
    let env = Env("root, root ALL=(ALL:ALL) NOPASSWD: ALL").build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()
}

#[test]
fn double_negative_is_positive() -> Result<()> {
    let env = Env("!!root ALL=(ALL:ALL) NOPASSWD: ALL")
        .user(USERNAME)
        .build()?;

    Command::new("sudo")
        .arg("true")
        .exec(&env)?
        .assert_success()
}

#[test]
fn negation_excludes_group_members() -> Result<()> {
    let env = Env("%users, !ghost ALL=(ALL:ALL) ALL")
        // use PAM to avoid `ghost` getting a password prompt
        .file("/etc/pam.d/sudo", PAMD_SUDO_PAM_PERMIT)
        // the primary group of all new users is `users`
        .user("ferris")
        .user("ghost")
        .build()?;

    Command::new("sudo")
        .arg("true")
        .as_user("ferris")
        .exec(&env)?
        .assert_success()?;

    let output = Command::new("sudo")
        .arg("true")
        .as_user("ghost")
        .exec(&env)?;

    assert!(!output.status().success());
    assert_eq!(Some(1), output.status().code());

    if sudo_test::is_original_sudo() {
        assert_contains!(output.stderr(), "ghost is not in the sudoers file");
    }

    Ok(())
}

#[test]
fn negation_is_order_sensitive() -> Result<()> {
    // negated items at the start of a specifier list  are meaningless
    let env = Env("!ghost, %users ALL=(ALL:ALL) NOPASSWD: ALL")
        // the primary group of all new users is `users`
        .user("ferris")
        .user("ghost")
        .build()?;

    Command::new("sudo")
        .arg("true")
        .as_user("ferris")
        .exec(&env)?
        .assert_success()?;

    Command::new("sudo")
        .arg("true")
        .as_user("ghost")
        .exec(&env)?
        .assert_success()
}

#[test]
fn user_alias_works() -> Result<()> {
    let env = Env("
User_Alias ADMINS = %users, !ghost
ADMINS ALL=(ALL:ALL) ALL")
    // use PAM to avoid password prompts
    .file("/etc/pam.d/sudo", PAMD_SUDO_PAM_PERMIT)
    // the primary group of all new users is `users`
    .user("ferris")
    .user("ghost")
    .build()?;

    Command::new("sudo")
        .arg("true")
        .as_user("ferris")
        .exec(&env)?
        .assert_success()?;

    let output = Command::new("sudo")
        .arg("true")
        .as_user("ghost")
        .exec(&env)?;

    assert!(!output.status().success());
    assert_eq!(Some(1), output.status().code());

    if sudo_test::is_original_sudo() {
        assert_contains!(output.stderr(), "ghost is not in the sudoers file");
    }

    Ok(())
}

#[ignore]
#[test]
fn negated_user_alias_works() -> Result<()> {
    let env = Env("
User_Alias ADMINS = %users, !ghost
!ADMINS ALL=(ALL:ALL) ALL")
    // use PAM to avoid password prompts
    .file("/etc/pam.d/sudo", PAMD_SUDO_PAM_PERMIT)
    // the primary group of all new users is `users`
    .user("ferris")
    .user("ghost")
    .build()?;

    Command::new("sudo")
        .arg("true")
        .as_user("ghost")
        .exec(&env)?
        .assert_success()?;

    let output = Command::new("sudo")
        .arg("true")
        .as_user("ferris")
        .exec(&env)?;

    assert!(!output.status().success());
    assert_eq!(Some(1), output.status().code());

    if sudo_test::is_original_sudo() {
        assert_contains!(output.stderr(), "ferris is not in the sudoers file");
    }

    Ok(())
}
