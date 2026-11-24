//! `lume` as the program the SSH client asks through, against an endpoint that answers
//! as the test says.

use std::ffi::OsStr;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use lume_ipc::{Endpoint, Listener};
use lume_proto::app::{
    APP_PROTO_VERSION, ASKPASS_ENDPOINT_VAR, ASKPASS_TOKEN_VAR, AppError, AppMessage, AppReply,
    AppReplyBody, AppRequest, AppRequestBody, AppResponse,
};
use lume_proto::{ControlDecoder, Secret};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "0123456789abcdef";

fn name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nth = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("test-askpass-{}-{nth}", std::process::id())
}

/// Runs `lume` as the SSH client does.
fn lume(endpoint: &str, args: &[&str], hint: Option<&str>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lume"));
    command
        .args(args)
        .env(ASKPASS_ENDPOINT_VAR, endpoint)
        .env(ASKPASS_TOKEN_VAR, TOKEN)
        .env_remove("SSH_ASKPASS_PROMPT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(hint) = hint {
        command.env("SSH_ASKPASS_PROMPT", hint);
    }
    command
}

/// What the endpoint was asked, and what `lume` made of the answer.
struct Seen {
    asked: AppRequestBody,
    output: Output,
}

/// Runs `lume` against an endpoint that answers the question with `answer`.
async fn asked(args: &[&str], hint: Option<&str>, answer: Result<AppReplyBody, AppError>) -> Seen {
    let name = name();
    let mut listener = Listener::bind(&Endpoint::named(&name).unwrap()).unwrap();
    let mut command = lume(&name, args, hint);
    let running = tokio::task::spawn_blocking(move || command.output().unwrap());
    let mut io = listener.accept().await.unwrap();
    let mut decoder = ControlDecoder::new();
    let mut chunk = [0; 4096];
    let mut answers = vec![
        answer,
        Ok(AppReplyBody::Hello {
            proto: APP_PROTO_VERSION,
            app_version: "0".to_owned(),
            pid: 1,
        }),
    ];
    let mut asked = None;
    while let Some(answer) = answers.pop() {
        let json = loop {
            if let Some(json) = decoder.next().unwrap() {
                break json;
            }
            let read = io.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0, "the question is asked");
            decoder.push(&chunk[..read]);
        };
        let AppMessage::Request(AppRequest { id, body }) = AppMessage::decode(&json).unwrap()
        else {
            panic!("not a request");
        };
        asked = Some(body);
        let mut frames = Vec::new();
        AppMessage::Response(AppResponse {
            id,
            result: answer.map(|body| AppReply {
                effect: String::new(),
                body,
            }),
        })
        .encode(&mut frames)
        .unwrap();
        io.write_all(&frames).await.unwrap();
    }
    Seen {
        asked: asked.unwrap(),
        output: running.await.unwrap(),
    }
}

fn secret(text: &str) -> Result<AppReplyBody, AppError> {
    Ok(AppReplyBody::Secret {
        secret: Some(Secret::new(text.to_owned())),
    })
}

fn question(prompt: &str, hint: Option<&str>) -> AppRequestBody {
    AppRequestBody::Askpass {
        token: Secret::new(TOKEN.to_owned()),
        prompt: prompt.to_owned(),
        hint: hint.map(str::to_owned),
    }
}

#[tokio::test]
async fn the_clients_words_are_asked_and_the_answer_is_printed() {
    let words = "The authenticity of host 'build (10.0.0.4)' can't be established.\n\
         Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
    let seen = asked(&[words], None, secret("yes")).await;
    assert_eq!(seen.asked, question(words, None));
    assert_eq!(seen.output.stdout, b"yes\n");
    assert!(seen.output.status.success());

    let seen = asked(&["Allow?"], Some("confirm"), secret("pá ss\"wörd")).await;
    assert_eq!(seen.asked, question("Allow?", Some("confirm")));
    assert_eq!(seen.output.stdout, "pá ss\"wörd\n".as_bytes());
}

#[tokio::test]
async fn words_that_look_like_an_option_or_a_command_are_still_only_words() {
    for words in [
        "--version",
        "-h",
        "ctl",
        "plugin",
        "config",
        "nothing-known",
        "",
    ] {
        let seen = asked(&[words], None, secret("x")).await;
        assert_eq!(seen.asked, question(words, None));
        assert_eq!(seen.output.stdout, b"x\n", "{words}");
    }
}

#[tokio::test]
async fn without_an_answer_nothing_is_printed() {
    let refused = [
        Ok(AppReplyBody::Secret { secret: None }),
        Err(AppError::new("not_found", "nothing is being signed in to")),
        Ok(AppReplyBody::Killed {}),
    ];
    for answer in refused {
        let seen = asked(&["Password: "], None, answer).await;
        assert_eq!(seen.output.status.code(), Some(1));
        assert!(seen.output.stdout.is_empty() && seen.output.stderr.is_empty());
    }
    // Nothing serves the endpoint.
    let output = lume(&name(), &["Password: "], None).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[test]
fn run_by_anyone_else_it_is_the_command_it_always_was() {
    for missing in [ASKPASS_ENDPOINT_VAR, ASKPASS_TOKEN_VAR] {
        let output = lume("nowhere", &["--version"], None)
            .env_remove(missing)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            output
                .stdout
                .starts_with(OsStr::new("lume ").as_encoded_bytes())
        );
    }
}
