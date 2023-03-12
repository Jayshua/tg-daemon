#![feature(try_blocks, slice_take)]

mod telegram_api;

use clap::Parser;
use tracing::{info, error, debug, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::collections::HashMap;
use telegram_api::*;




// Constants




/// Alphabet used to randomly generate the temporary filenames for files downloaded from Telegram
const FILE_ID_ALPHABET: [char; 62] = [
	'a','b','c','d','e','f','g','h','i','j','k','l','m','n','o','p','q','r','s','t','u','v','w','x','y','z',
	'A','B','C','D','E','F','G','H','I','J','K','L','M','N','O','P','Q','R','S','T','U','V','W','X','Y','Z',
	'0','1','2','3','4','5','6','7','8','9'
];


/// How long to poll telegram before closing the HTTP connection and re-opening it
/// Should be some reasonably large value - Telegram prefers bots don't "refresh" HTTP connections too frequently.
const TG_TIMEOUT: u64 = 300;




// Types




/// Arguments that can be passed to the program
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
	/// Path to the executable to spawn and send messages to
	#[arg(short, long)]
	execute: String,


	/// ID of the telegram bot to listen for messages to.
	///
	/// You can get this from the BotFather (https://telegram.me/BotFather)
	#[arg(short, long)]
	bot_id: String,


	/// Whitelist chat ids. Unauthorized chat ids will not spawn a handler process.
	///
	/// You'll probably need to run the daemon without this at least once to
	/// figure out the id of the chat you want to use here.
	#[arg(long)]
	chat_id: Vec<u64>,


	/// Suppress chat message upon handler failure
	///
	/// By default, the daemon will send a notification error message to the telegram chat
	/// if the handler process terminates with a non-zero status code.
	/// Setting this to false will cause the daemon to not send a message if the handler process
	/// terminates with a non-zero status code.
	#[arg(long)]
	suppress_handler_error: bool,


	/// Base URL to access the Telegram API at.
	///
	/// If you're connecting to the telegram bot development server, you can do that here.
	#[arg(long, default_value = "https://api.telegram.org")]
	tg_api_url: String,


	/// Send the first command when spawning a process to stdin
	///
	/// When processing a new message, if a process is not running for the chat the message was sent in,
	/// a new process will be spawned and the first message provided to the proces via standard environment arguments
	/// as though the process were called from the command line like: `my-executable /command from telegram message`.
	/// For as long as the process remains alive, subsequent arguments will be sent to the process's stdin.
	///
	/// If the process is designed to run indefinitely, it could be inconvenient to handle the first message
	/// via environment arguments. This flag will cause the first message to be passed to the process via stdin,
	/// in the same way subsequent arguments would.
	#[arg(long)]
	pipe_first_message: bool,


	/// File containing commands supported by the bot.
	///
	/// The file should contain one command per line starting with the text of the command
	/// (without a leading slash) followed by any number of spaces or tabs, then the
	/// documentation of the command.
	///
	/// This file will be used to tell telegram what commands the bot supports, which telegram
	/// will then use to generate a menu button in the app.
	#[arg(long)]
	commands_file: Option<String>,
}



/// Struct to contain context needed for talking to telegram
///
/// This data is used by basically every function that needs to talk to telegram,
/// so it's packaged up into a nice little struct.
/// Marked public so that tracing can emit the bot_id in log messages.
#[derive(Debug, Clone)]
pub struct TgClient {
	client: reqwest::Client,
	base_url: String,
	bot_id: String,
}

impl TgClient {
	/// Construct the normal url prefix used when communicating with Telegram
	/// Contains the telegram base url and the bot id
	fn bot_base(&self) -> String {
		format!("{}/bot{}", self.base_url, self.bot_id)
	}
}



/// Telegram events that can be sent to the tokio thread that handles a particular chat process
#[derive(Debug)]
enum HandleEvent {
	/// A regular message from the Telegram user
	Message(Message),
	/// The Telegram user tapped on an inline keyboard button
	Callback(CallbackQuery),
}



/// Errors that can occur when handling messages to/from a handler process
#[derive(Debug, derive_enum_from_into::EnumFrom)]
enum HandleError {
	UnclosedHeredoc,
	EditedUnsentMessage,
	DeletedUnsentMessage,
	RemovedInlineKeyboardForUnsetMessage,
	Reqwest(reqwest::Error),
	Utf8Error(std::str::Utf8Error),
	TelegramError(TelegramError),
	IoError(std::io::Error),
	SendFile(SendFileError),
	SendMessage(TgRequestError),
	DownloadFileError(DownloadFileError),
	InlineButtonExpectedKind,
	InlineButtonExpectedData,
	InvalidInlineButtonKind(String),
}




// Functions




#[tokio::main]
async fn main() {
	let args = Args::parse();

	let tracing_subscriber = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_env("LOG_LEVEL"))
		.finish();

	tracing::subscriber::set_global_default(tracing_subscriber).expect("Setting the tracing subscriber should not fail");

	poll_telegram(args).await;
}



/// Poll telegram for updates, spawning new processes to handle them as needed
/// Will also update the bot's command list when first polled
#[tracing::instrument(skip_all)]
async fn poll_telegram(args: Args) {
	let tg = TgClient {
		client: reqwest::Client::new(),
		base_url: args.tg_api_url.clone(),
		bot_id: args.bot_id.clone(),
	};


	if let Some(commands_path) = &args.commands_file {
		info!(commands_path, "Setting bot commands from file");
		let set_result = setup_commands(tg.clone(), commands_path).await;
		if let Err(reason) = set_result {
			error!(?reason, "Failed to set commands from file.");
			return;
		}
	}


	let mut chat_handlers: HashMap<u64, tokio::sync::mpsc::Sender<HandleEvent>> = HashMap::new();
	let mut poll_failures = 0;
	let mut next_update_id = 0;
	let bot_base = tg.bot_base();
	loop {
		#[derive(Debug, derive_enum_from_into::EnumFrom)]
		enum GetUpdateError {
			TelegramError(TelegramError),
			ReqwestError(reqwest::Error),
		}


		let result: Result<Vec<UpdateResponse>, GetUpdateError> = try {
			debug!(next_update_id, poll_failures, "Polling telegram");

			tg.client
				.get(format!("{bot_base}/getUpdates?offset={next_update_id}&timeout={TG_TIMEOUT}&allowed_updates=[\"message\",\"callback_query\"]"))
				.timeout(std::time::Duration::from_secs(TG_TIMEOUT + 1))
				.send().await?
				.json::<TelegramResponse<Vec<UpdateResponse>>>().await?
				.to_result()?
		};


		match result {
			// Network error contacting telegram, use an exponential backoff to sleep before retrying.
			Err(reason) => {
				poll_failures = std::cmp::min(poll_failures + 1, 5);
				let sleep_duration = u64::pow(2, poll_failures);
				error!(?reason, "Failed to poll telegram for updates. Sleeping for {} seconds.", sleep_duration);
				tokio::time::sleep(std::time::Duration::from_secs(sleep_duration)).await;
			}

			Ok(updates) => {
				poll_failures = 0;

				// Telegram can deliver more than one update at a time
				for update in updates {
					next_update_id = std::cmp::max(next_update_id, update.update_id + 1);

					let (chat_id, event) = match update {
						UpdateResponse { message: Some(message), .. } =>
							(message.chat.id, HandleEvent::Message(message)),

						UpdateResponse { callback_query: Some(callback), .. } =>
							(callback.message.chat.id, HandleEvent::Callback(callback)),

						_ =>
							panic!("Telegram promised to always return a message or callback query!"),
					};

					if args.chat_id.len() > 0 && !args.chat_id.contains(&chat_id) {
						warn!(chat_id, "Ignoring non-whitelisted chat");
						continue;
					}

					debug!(chat_id, "Received message from telegram");

					// Careful not to drop a message if the old chat handler crashed or something
					let unsent_event = match chat_handlers.get(&chat_id) {
						None => Some(event),
						Some(sender) => {
							match sender.send(event).await {
								Ok(()) => None,
								Err(tokio::sync::mpsc::error::SendError(event)) => Some(event),
							}
						}
					};

					// The handler process either hasn't been created or was terminated
					if let Some(event) = unsent_event {
						info!(chat_id, "Spawning new handler process");
						let (sender, receiver) = tokio::sync::mpsc::channel(25);
						sender.send(event).await.expect("A new sender should never fail");
						chat_handlers.insert(chat_id, sender);
						tokio::spawn(chat_handler(tg.clone(), args.clone(), chat_id, receiver));
					}
				}
			}
		}
	}
}



/// Spawn a new handler process for a telegram chat
/// Will loop processing input from the handler process and messages from the provided receiver until
/// the handler process terminates or a fatal error is encountered.
#[tracing::instrument(skip(tg, config, receiver))]
async fn chat_handler(tg: TgClient, config: Args, chat_id: u64, mut receiver: tokio::sync::mpsc::Receiver<HandleEvent>) {
	let args: Vec<String> =
		if !config.pipe_first_message {
			let first_message = receiver.recv().await.expect("sender should not be dropped until chat_handler terminates");
			event_to_args(&first_message, true).await
		} else {
			vec![]
		};

	let child = tokio::process::Command::new(config.execute)
		.args(args)
		.stdout(std::process::Stdio::piped())
		.stdin(std::process::Stdio::piped())
		.spawn();

	let mut child = match child {
		Err(reason) => {
			error!(?reason, "Unable to spawn handler process");
			return;
		}

		Ok(child) => child,
	};


	let mut stdout = child.stdout.take().expect("New child process should have stdout");
	let mut stdin = child.stdin.take().expect("New child process should have stdin");
	let mut stdout_buffer = [0u8; 1024];
	let mut message_buffer = String::new();
	let mut next_message_keyboard = Vec::new();
	let mut last_message_id = None;

	let process_result: Result<std::process::ExitStatus, HandleError> = try { 'outer: loop {
		tokio::select! {
			// Forward messages from telegram to the handler
			message = receiver.recv() => {
				let message = message.expect("sender should not drop until chat_handler terminates");
				let mut args = event_to_args(&message, false).await;
				args.push("\n".to_string());
				let args = args.join(" ");
				stdin.write(args.as_bytes()).await?;
			}

			// Accept messages from the handler, handling some in the daemon
			// and forwarding others to Telegram.
			read_result = stdout.read(&mut stdout_buffer) => {
				let bytes_read = read_result?;

				// Reading 0 bytes indicates the child process has terminated
				if bytes_read == 0 {
					drop(stdin);
					drop(stdout);
					let exit_status = child.wait().await?;
					break 'outer exit_status;
				}

				let data = &stdout_buffer[..bytes_read];
				let data = std::str::from_utf8(data)?;
				let mut line_iterator = data.lines();
				while let Some(line) = line_iterator.next() {
					if line.starts_with("//heredoc") {
						let terminator = &line[10..].trim().to_string();
						debug!(terminator, "Received //heredoc");

						// Loop writing data to message_buffer until the terminator
						// is found at the start of a line.
						'heredoc: loop {
							while let Some(line) = line_iterator.next() {
								if line.starts_with(terminator) {
									break 'heredoc;
								} else {
									message_buffer.push_str(line);
									message_buffer.push_str("\n");
								}
							}

							let bytes_read = stdout.read(&mut stdout_buffer).await?;

							if bytes_read == 0 {
								Err(HandleError::UnclosedHeredoc)?;
							}

							let data = &stdout_buffer[..bytes_read];
							let data = std::str::from_utf8(data)?;
							line_iterator = data.lines();
						}
					}

					else if line.starts_with("//send-file") {
						debug!("Received //send-file");
						let file_path = &line[12..].trim();
						send_file(tg.clone(), chat_id, file_path).await?;
					}

					else if line.starts_with("//send-photo") {
						debug!("Received //send-photo");
						let file_path = &line[13..].trim();
						send_photo(tg.clone(), chat_id, file_path).await?;
					}

					else if line.starts_with("//chat-action") {
						debug!("Received //chat-action");
						let action = &line[14..].trim();
						send_chat_action(tg.clone(), chat_id, action).await?;
					}

					else if line.starts_with("//download-file") {
						debug!("Received //download-file");
						let file_id = &line[16..].trim();
						let file_path = download_file(tg.clone(), chat_id, file_id).await?;
						let file_path = file_path.display();
						stdin.write(format!("//tg-file-download {file_path}\n").as_bytes()).await?;
					}

					else if line.starts_with("//inline-button") {
						debug!("Received //inline-button");

						let (kind, line) = split_quoted(line[15..].trim()).ok_or(HandleError::InlineButtonExpectedKind)?;
						let (data, line) = split_quoted(line).ok_or(HandleError::InlineButtonExpectedData)?;

						let button = match kind.as_str() {
							"url" => InlineKeyboardButton { text: line.to_string(), variant: InlineKeyboardVariant::Url(data), },
							"callback" => InlineKeyboardButton { text: line.to_string(), variant: InlineKeyboardVariant::Callback(data), },
							kind => Err(HandleError::InvalidInlineButtonKind(kind.to_string()))?,
						};

						next_message_keyboard.push(button);
					}

					else if line.starts_with("//delete") {
						delete_message(tg.clone(), chat_id, last_message_id.ok_or(HandleError::DeletedUnsentMessage)?).await?;
						last_message_id = None;
					}

					else if line.starts_with("//remove-inline-keyboard") {
						debug!("Received //remove-inline-keyboard");

						send_message(
							tg.clone(),
							chat_id,
							Some(last_message_id.ok_or(HandleError::RemovedInlineKeyboardForUnsetMessage)?),
							None::<&str>,
							&[]
						)
						.await?;
					}

					else if line.starts_with("//edit") {
						debug!("Received //edit");

						send_message(
							tg.clone(),
							chat_id,
							Some(last_message_id.ok_or(HandleError::EditedUnsentMessage)?),
							if message_buffer.len() > 0 {
								Some(&message_buffer)
							} else {
								None
							},
							&next_message_keyboard
						)
						.await?;

						next_message_keyboard.clear();
						message_buffer.clear();
					}

					else if line.starts_with("//send") {
						debug!("Received //send");

						if message_buffer.len() == 0 {
							warn!("Tried to //send, but the send buffer was empty! Write some content to stdout.");
						} else {
							let message = send_message(tg.clone(), chat_id, None, Some(&message_buffer), &next_message_keyboard).await?;
							message_buffer.clear();
							next_message_keyboard.clear();
							last_message_id = Some(message.message_id);
						}
					}

					else {
						message_buffer.push_str(line);
						message_buffer.push_str("\n");
					}
				}
			}
		}
	} };


	match process_result {
		Ok(exit_status) if exit_status.success() => {
			info!("Handler process ended successfully");

			if message_buffer.len() > 0 && message_buffer != "\n" {
				debug!("Sending remainder of handler process stdout");

				if let Err(reason) = send_message(tg, chat_id, None, Some(&message_buffer), &next_message_keyboard).await {
					error!(?reason, "Error sending remainder of handler process stdout");
				}
			}
		}

		Ok(exit_status) => {
			error!(?exit_status, "Handler process terminated abnormally");

			if !config.suppress_handler_error {
				if let Err(reason) = send_message(tg, chat_id, None, Some("Fatal Server Error"), &next_message_keyboard).await {
					error!(?reason, "Error sending crash notification to telegram client");
				}
			}
		}

		Err(reason) => {
			error!(?reason, "Fatal error");
		}
	}
}



/// Convert a Telegram message into a command+args vec of strings
///
/// Returns something like this as a vec of strings:
///    //tg-document --file-name photo.jpg --file-id 3klfjl2k3fjl23kj --mime-type image/jpg
///
async fn event_to_args(message: &HandleEvent, split_text_args: bool) -> Vec<String> {
	match message {
		HandleEvent::Callback(CallbackQuery { data, .. }) => {
			vec!["//tg-callback".to_string(), data.to_string()]
		}

		HandleEvent::Message(Message { text: Some(text), .. }) if split_text_args => {
			let text = safe_text(text);
			text.split_whitespace().map(str::to_string).collect::<Vec<String>>()
		}

		HandleEvent::Message(Message { text: Some(text), .. }) => {
			let text = safe_text(text);
			vec![text.to_string()]
		}

		HandleEvent::Message(Message { document: Some(document), .. }) => {
			let mut args = vec![
				"//tg-document".to_string(),
				"--file-id".to_string(),
				document.file_id.to_string()
			];

			match &document.unsafe_file_name {
				Some(unsafe_file_name) if unsafe_file_name != "" => {
					let safe_file_name = clean_file_name(&unsafe_file_name);
					args.push("--file-name".to_string());
					args.push(safe_file_name);
				}

				_ => {}
			}

			match &document.unsafe_mime_type {
				Some(unsafe_mime_type) if unsafe_mime_type != "" => {
					match unsafe_mime_type.parse::<mime::Mime>() {
						Err(_) => {}
						Ok(safe_mime_type) => {
							args.push("--mime-type".to_string());
							args.push(safe_mime_type.essence_str().to_string());
						}
					}
				}

				_ => {}
			}

			args
		}

		HandleEvent::Message(Message { photo: Some(photo_sizes), .. }) => {
			let mut photo_sizes: Vec<_> = photo_sizes.to_vec();
			photo_sizes.sort_by_key(|size| size.width * size.height);

			let mut args = vec!["//tg-photo".to_string()];
			for size in photo_sizes {
				args.push(size.file_id.to_string());
				args.push(size.width.to_string());
				args.push(size.height.to_string());
			}

			args
		}

		_ => {
			error!("Error processing telegram message - unknown message type");
			vec!["//tg-unknown".to_string()]
		}
	}
}



/// Get the first space-separated "argument" from a string, returning the rest of the string unchanged.
///
/// Handles quotes around arguments containing spaces, and escaping quotes with the backslash character.
///
/// Examples:
///    first second third
///     => ("first", " second third")
///
///    "first wi\"th spaces" second third
///     => ("first with spaces", " second third")
///
/// Note that backslashes and quotes are not included in the returned argument.
///
fn split_quoted(string: &str) -> Option<(String, &str)> {
	let string = string.trim_start();

	let mut segment = String::new();
	let mut is_escaping = false;
	let mut is_quoting = false;
	for (index, character) in string.char_indices() {
		if is_escaping {
			is_escaping = false;
			segment.push(character);
		}
		else if character == '\\' {
			is_escaping = true;
		}
		else if character == '"' {
			is_quoting = !is_quoting;
		}
		else if character == ' ' && !is_quoting {
			return Some((segment, &string[index..]));
		} else {
			segment.push(character);
		}
	}

	if segment.len() > 0 {
		return Some((segment, &string[string.len()..string.len()]));
	} else {
		return None;
	}
}

/// Tests for the split_quoted function
#[cfg(test)]
#[test]
fn test_quote_splitting() {
	let valid_cases = &[
		("first",                                ("first", "")),
		("    first",                            ("first", "")),
		("    first   ",                         ("first", "   ")),
		("first second",                         ("first", " second")),
		("first second third",                   ("first", " second third")),
		("first \"second third\"",               ("first", " \"second third\"")),
		("\"first with spaces\" second",         ("first with spaces", " second")),
		("    first    second   ",               ("first", "    second   ")),
		("    first second   ",                  ("first", " second   ")),
		(r#" "fir\"st with quote" remainder  "#, ("fir\"st with quote", " remainder  ")),
	];

	assert_eq!(split_quoted(""), None);

	for &(input, (prefix, suffix)) in valid_cases {
		assert_eq!(split_quoted(input), Some((prefix.to_string(), suffix)));
	}
}



/// Collapse multiple leading '/' into a single '/'
///
/// Prevents the client from impersonating the daemon to the handler process
/// by, e.g., sending `//tg-document ...`
///
fn safe_text(mut input: &str) -> &str {
	while input.starts_with("//") {
		input = &input[1..];
	}

	input
}



/// Make a string safe for including in a space separated list of arguments
///
/// Removes all characters that are not [a-zA-Z0-9_.]
///
fn clean_file_name(input: &str) -> String {
	input
		.chars()
		.filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
		.collect::<String>()
}
