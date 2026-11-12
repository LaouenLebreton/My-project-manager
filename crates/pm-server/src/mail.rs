//! Sending email: invitations, when the server has a mail account (SMTP settings). Without one
//! the people inviting copy each invite and send it themselves.

use std::time::Duration;

use lettre::message::Mailbox;
use lettre::{Message, SmtpTransport, Transport};

/// Something that sends plain-text email. Blocking: call it off the async threads.
pub trait Mailer: Send + Sync {
    /// Sends one message; the error is in plain words, for the person inviting.
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String>;
}

/// A mail account reached over SMTP.
pub struct Smtp {
    transport: SmtpTransport,
    from: Mailbox,
}

impl Smtp {
    /// From an SMTP address such as `smtps://user:password@smtp.example.com` (TLS from the
    /// start, port 465) or `smtp://user:password@smtp.example.com:587?tls=required` (STARTTLS),
    /// and the sender, such as `Super PM <pm@example.com>`.
    pub fn new(url: &str, from: &str) -> Result<Self, String> {
        let from: Mailbox = from.parse().map_err(|e| format!("the sender address “{from}” is not valid: {e}"))?;
        let transport = SmtpTransport::from_url(url).map_err(|e| format!("the SMTP address is not valid: {e}"))?.timeout(Some(Duration::from_secs(20))).build();
        Ok(Smtp { transport, from })
    }

    /// Checks that the mail server answers and accepts the account.
    pub fn check(&self) -> Result<(), String> {
        match self.transport.test_connection() {
            Ok(true) => Ok(()),
            Ok(false) => Err("the mail server did not answer".into()),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl Mailer for Smtp {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        let to: Mailbox = to.parse().map_err(|_| "not an email address".to_owned())?;
        let message = Message::builder().from(self.from.clone()).to(to).subject(subject).body(body.to_owned()).map_err(|e| e.to_string())?;
        self.transport.send(&message).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// The invite a new person gets: the text they paste into Team > Join a Team (the same format
/// the app builds when invites are copied by hand).
pub fn invite_text(inviter: &str, team: &str, server: &str, email: &str, code: &str) -> (String, String) {
    let subject = format!("{inviter} invited you to {team} in Super PM");
    let body = format!(
        "{inviter} invited you to the team “{team}” in Super PM, to plan projects together.\n\n\
         1. Install Super PM if you do not have it: https://github.com/LaouenLebreton/My-project-manager/releases\n\
         2. Open it and choose Team > Join a Team.\n\
         3. Paste this invite (the whole line):\n\n\
         SuperPM-Invite|{server}|{email}|{code}\n\n\
         The invite works once and signs you in. Do not forward it.\n"
    );
    (subject, body)
}

/// The message for someone who already has an account: they were added to the team.
pub fn added_text(inviter: &str, team: &str) -> (String, String) {
    let subject = format!("{inviter} added you to {team} in Super PM");
    let body = format!("{inviter} added you to the team “{team}” in Super PM. Its projects are in Team > Open Team Project.\n");
    (subject, body)
}
