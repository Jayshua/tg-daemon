#![feature(try_blocks, slice_take)]

use clap::Parser;
use tracing::{info, error, debug, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::collections::HashMap;



/// Alphabet used to randomly generate the temporary filenames for files downloaded from Telegram
const FILE_ID_ALPHABET: [char; 62] = [
	'a','b','c','d','e','f','g','h','i','j','k','l','m','n','o','p','q','r','s','t','u','v','w','x','y','z','A','B','C','D','E','F','G','H','I','J','K','L','M','N','O','P','Q','R','S','T','U','V','W','X','Y','Z','0','1','2','3','4','5','6','7','8','9'
];



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
/// This data is used by basically every function that needs to talk to telegram,
/// so it's packaged up into a nice little struct.
#[derive(Debug, Clone)]
struct TgClient {
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







#[tokio::main]
async fn main() {
	let args = Args::parse();

	let tracing_subscriber = tracing_subscriber::FmtSubscriber::builder().finish();

	tracing::subscriber::set_global_default(tracing_subscriber).expect("Setting the tracing subscriber should not fail");

	poll_telegram(args).await;
}





/// Poll telegram for updates, spawning new processes to handle them as needed
/// Also update's the bots command list when first started
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


	let mut chat_handlers: HashMap<u64, tokio::sync::mpsc::Sender<Message>> = HashMap::new();
	let mut poll_failures = 0;
	let mut next_update_id = 0;
	let bot_base = tg.bot_base();
	let timeout = 300;
	loop {
		let update_url = format!("{bot_base}/getUpdates?offset={next_update_id}&timeout={timeout}&allowed_updates=[\"message\"]");

		#[derive(Debug, derive_enum_from_into::EnumFrom)]
		enum GetUpdateError {
			TelegramError(TelegramError),
			ReqwestError(reqwest::Error),
		}

		let result: Result<Vec<UpdateResponse>, GetUpdateError> = try {
			info!(next_update_id, poll_failures, "Polling telegram");

			tg.client.get(update_url)
				.timeout(std::time::Duration::from_secs(timeout + 1))
				.send().await?
				.json::<TelegramResponse<Vec<UpdateResponse>>>().await?
				.to_result()?
		};

		match result {
			Err(reason) => {
				poll_failures = std::cmp::min(poll_failures + 1, 5);
				let sleep_duration = u64::pow(2, poll_failures);
				error!(?reason, "Failed to poll telegram for updates. Sleeping for {} seconds.", sleep_duration);
				tokio::time::sleep(std::time::Duration::from_secs(sleep_duration)).await;
			}

			Ok(updates) => {
				poll_failures = 0;

				for update in updates {
					next_update_id = std::cmp::max(next_update_id, update.update_id + 1);

					let chat_id = update.message.chat.id;

					if args.chat_id.len() > 0 && !args.chat_id.contains(&chat_id) {
						warn!(chat_id, "Ignoring non-whitelisted chat");
						continue;
					}

					info!(chat_id, "Received message from telegram");

					let unsent_message = match chat_handlers.get(&chat_id) {
						None => Some(update.message),
						Some(sender) => {
							match sender.send(update.message).await {
								Ok(()) => None,
								Err(tokio::sync::mpsc::error::SendError(message)) => Some(message),
							}
						}
					};

					if let Some(message) = unsent_message {
						info!(chat_id, "Spawning new handler process");
						let (sender, receiver) = tokio::sync::mpsc::channel(25);
						sender.send(message).await.expect("A new sender should never fail");
						chat_handlers.insert(chat_id, sender);
						tokio::spawn(chat_handler(tg.clone(), args.clone(), chat_id, receiver));
					}
				}
			}
		}
	}
}







/// Errors that can occur when handling messages to/from a handler process
#[derive(Debug, derive_enum_from_into::EnumFrom)]
enum HandleError {
	UnclosedHeredoc,
	Reqwest(reqwest::Error),
	Utf8Error(std::str::Utf8Error),
	TelegramError(TelegramError),
	IoError(std::io::Error),
	SendFile(SendFileError),
	SendMessage(SendMessageError),
	SendChatAction(SendChatActionError),
	DownloadFileError(DownloadFileError),
}


/// Spawn a new handler process for a telegram chat
/// Will loop processing input from the handler process and messages from the provided receiver until
/// the handler process terminates or a fatal error is encountered.
#[tracing::instrument(skip(tg, config, receiver))]
async fn chat_handler(tg: TgClient, config: Args, chat_id: u64, mut receiver: tokio::sync::mpsc::Receiver<Message>) {
	let args: Vec<String> =
		if !config.pipe_first_message {
			let first_message = receiver.recv().await.expect("sender should not be dropped until chat_handler terminates");
			message_to_args(&first_message, true).await
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

	let process_result: Result<std::process::ExitStatus, HandleError> = try { 'outer: loop {
		tokio::select! {
			message = receiver.recv() => {
				let message = message.expect("sender should not drop until chat_handler terminates");
				let mut args = message_to_args(&message, false).await;
				args.push("\n".to_string());
				let args = args.join(" ");
				stdin.write(args.as_bytes()).await?;
			}

			read_result = stdout.read(&mut stdout_buffer) => {
				let bytes_read = read_result?;

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

					else if line.starts_with("//send") {
						debug!("Received //send");

						if message_buffer.len() == 0 {
							warn!("Tried to //send, but the send buffer was empty! Write some content to stdout.");
						} else {
							send_message(tg.clone(), chat_id, &message_buffer).await?;
							message_buffer.clear();
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
				info!("Sending remainder of handler process stdout");

				if let Err(reason) = send_message(tg, chat_id, &message_buffer).await {
					error!(?reason, "Error sending remainder of handler process stdout");
				}
			}
		}

		Ok(exit_status) => {
			error!(?exit_status, "Handler process terminated abnormally");

			if !config.suppress_handler_error {
				if let Err(reason) = send_message(tg, chat_id, "Fatal Server Error").await {
					error!(?reason, "Error sending crash notification to telegram client");
				}
			}
		}

		Err(reason) => {
			error!(?reason, "Fatal error");
		}
	}
}



/// Convert a Telegram message into a command+args like vec of strings
///
/// Like this: //tg-document --file-name photo.jpg --file-id 3klfjl2k3fjl23kj --mime-type image/jpg
async fn message_to_args(message: &Message, split_text_args: bool) -> Vec<String> {
	match message {
		Message { text: Some(text), .. } if split_text_args => {
			let text = safe_text(text);
			text.split_whitespace().map(str::to_string).collect::<Vec<String>>()
		}

		Message { text: Some(text), .. } => {
			let text = safe_text(text);
			vec![text.to_string()]
		}

		Message { document: Some(document), .. } => {
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

		Message { photo: Some(photo_sizes), .. } => {
			let mut args = vec!["//tg-photo".to_string()];

			let photo_sizes = &mut &photo_sizes[..];
			while let Some(photo_size) = photo_sizes.take_first() {
				args.push(photo_size.file_id.to_string());
				args.push(photo_size.width.to_string());
				args.push(photo_size.height.to_string());
			}

			args
		}

		_ => {
			error!("Error processing telegram message - unknown message type");
			vec!["//tg-unknown".to_string()]
		}
	}
}










#[derive(Debug, derive_enum_from_into::EnumFrom)]
enum SendMessageError {
	Reqwest(reqwest::Error),
	TelegramError(TelegramError),
}

#[tracing::instrument(skip_all)]
async fn send_message(tg: TgClient, chat_id: u64, message: &str) -> Result<Message, SendMessageError> {
	let message = tg.client
		.post(format!(r#"{}/sendMessage"#, tg.bot_base()))
		.json(&serde_json::json!({
			"chat_id": chat_id,
			"text": message
		}))
		.send().await?
		.json::<TelegramResponse<Message>>().await?
		.to_result()?;

	Ok(message)
}





#[derive(Debug, derive_enum_from_into::EnumFrom)]
enum SendFileError {
	FileIo(std::io::Error),
	Reqwest(reqwest::Error),
	Telegram(TelegramError),
}

#[tracing::instrument(skip(tg))]
async fn send_file(tg: TgClient, chat_id: u64, file_path: impl AsRef<std::path::Path> + std::fmt::Debug) -> Result<Message, SendFileError> {
	let mut file = tokio::fs::File::open(file_path).await?;
	let mut file_buffer = Vec::new();
	file.read_to_end(&mut file_buffer).await?;

	let file_length: u64 = file_buffer.len() as u64;

	let photo_form_part = reqwest::multipart::Part::stream_with_length(file_buffer, file_length).file_name("document");
	let form = reqwest::multipart::Form::new()
		.text("chat_id", format!("{}", chat_id))
		.part("document", photo_form_part);

	let message = tg.client
		.post(format!("{}/sendDocument", tg.bot_base()))
		.multipart(form)
		.send().await?
		.json::<TelegramResponse<Message>>().await?
		.to_result()?;

	Ok(message)
}



#[tracing::instrument(skip(tg))]
async fn send_photo(tg: TgClient, chat_id: u64, file_path: impl AsRef<std::path::Path> + std::fmt::Debug) -> Result<Message, SendFileError> {
	let mut file = tokio::fs::File::open(file_path).await?;
	let mut file_buffer = Vec::new();
	file.read_to_end(&mut file_buffer).await?;

	let file_length: u64 = file_buffer.len() as u64;

	let photo_form_part = reqwest::multipart::Part::stream_with_length(file_buffer, file_length).file_name("photo");
	let form = reqwest::multipart::Form::new()
		.text("chat_id", format!("{}", chat_id))
		.part("photo", photo_form_part);

	let message = tg.client
		.post(format!("{}/sendPhoto", tg.bot_base()))
		.multipart(form)
		.send().await?
		.json::<TelegramResponse<Message>>().await?
		.to_result()?;

	Ok(message)
}



#[derive(Debug, derive_enum_from_into::EnumFrom)]
enum SendChatActionError {
	TelegramError(TelegramError),
	Reqwest(reqwest::Error),
}

#[tracing::instrument(skip(tg))]
async fn send_chat_action(tg: TgClient, chat_id: u64, action: &str) -> Result<(), SendChatActionError> {
	tg.client
		.post(format!("{}/sendChatAction", tg.bot_base()))
		.json(&serde_json::json!({
			"chat_id": chat_id,
			"action": action,
		}))
		.send().await?
		.json::<TelegramResponse<serde_json::Value>>().await?
		.to_result()?;

	Ok(())
}







#[derive(Debug, derive_enum_from_into::EnumFrom)]
enum DownloadFileError {
	Reqwest(reqwest::Error),
	FileIo(std::io::Error),
	TelegramError(TelegramError),
	FilePathMissing,
}


#[tracing::instrument(skip(tg))]
async fn download_file(tg: TgClient, chat_id: u64, file_id: &str) -> Result<std::path::PathBuf, DownloadFileError> {
	let file = tg.client
		.post(format!("{}/getFile", tg.bot_base()))
		.json(&serde_json::json!({"file_id": file_id}))
		.send().await?
		.json::<TelegramResponse<File>>().await?
		.to_result()?;

	let file_path = file.file_path.ok_or(DownloadFileError::FilePathMissing)?;

	let mut response = tg.client
		.get(format!("{}/file/bot{}/{file_path}", tg.base_url, tg.bot_id))
		.send().await?;

	let mut temp_file_path = std::env::temp_dir();
	temp_file_path.push(nanoid::nanoid!(12, &FILE_ID_ALPHABET));
	let mut file = tokio::fs::File::create(&temp_file_path).await?;
	while let Some(chunk) = response.chunk().await? {
		debug!("Writing file chunk to temp file");
		file.write(&chunk).await?;
	}

	Ok(temp_file_path)
}
































/// Collapse n leading '/' to a single '/'
///
/// Prevents the client from impersonating the daemon to the handler process by, e.g., sending `//tg-upload ...`
fn safe_text(mut input: &str) -> &str {
	while input.starts_with("//") {
		input = &input[1..];
	}

	input
}


/// The user-provided file name for document uploads is passed to the handler process
/// in a --command-arg style. This function cleans the user-provided file name, only
/// permitting these characters: [a-zA-Z0-9_.-].
fn clean_file_name(input: &str) -> String {
	input
		.chars()
		.filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
		.collect::<String>()
}








#[derive(Debug, derive_enum_from_into::EnumFrom)]
enum SetupCommandsError {
	FileIo(std::io::Error),
	ReqwestError(reqwest::Error),
	FileEmpty,
	InvalidCommandLine(usize),
	TelegramError(TelegramError),
}

#[tracing::instrument(skip_all)]
async fn setup_commands(tg: TgClient, commands_path: &str) -> Result<(), SetupCommandsError> {
	let mut file = tokio::fs::File::open(commands_path).await?;
	let mut buffer = String::new();
	file.read_to_string(&mut buffer).await?;

	let mut commands = Vec::new();
	for (line_index, line) in buffer.lines().enumerate() {
		let (command, description) = line.split_once(" ").ok_or(SetupCommandsError::InvalidCommandLine(line_index + 1))?;
		let (command, description) = (command.trim(), description.trim());

		if command.len() == 0 || description.len() == 0 {
			return Err(SetupCommandsError::InvalidCommandLine(line_index + 1));
		}

		commands.push(serde_json::json!({
			"command": command,
			"description": description,
		}));
	}

	if commands.len() == 0 {
		return Err(SetupCommandsError::FileEmpty);
	}

	tg.client
		.post(format!("{}/setMyCommands", tg.bot_base()))
		.json(&serde_json::json!({ "commands": commands }))
		.send().await?
		.json::<TelegramResponse<bool>>().await?
		.to_result()?;

	Ok(())
}













#[derive(Debug, serde::Deserialize)]
struct File {
	file_path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct UpdateResponse {
	update_id: u64,
	message: Message,
}

#[derive(Debug, serde::Deserialize)]
struct Message {
	text: Option<String>,
	chat: Chat,
	document: Option<Document>,
	photo: Option<Vec<PhotoSize>>,
}

#[derive(Debug, serde::Deserialize)]
struct User {

}

#[derive(Debug, serde::Deserialize)]
struct Document {
	file_id: String,

	#[serde(rename="file_name")]
	unsafe_file_name: Option<String>,

	#[serde(rename="mime_type")]
	unsafe_mime_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PhotoSize {
	file_id: String,
	width: u32,
	height: u32,
}

#[derive(Debug, serde::Deserialize)]
struct Chat {
	id: u64,
}









/// Success/Failure type returned by all Telegram requests
///
/// Telegram always returns JSON of the form { "ok": bool, "result": ...whatever the endpoint returns... }
/// with an optional "description" field that exists if "ok" is false.
/// This struct is that.
#[derive(Debug, serde::Deserialize)]
struct TelegramResponse<Data> {
	ok: bool,
	description: Option<String>,
	result: Option<Data>,
}

#[derive(Debug)]
struct TelegramError(String);

impl<Data> TelegramResponse<Data> {
	fn to_result(self) -> Result<Data, TelegramError> {
		if self.ok {
			Ok(self.result.expect("Ok telegram responses should have results"))
		} else {
			Err(TelegramError(self.description.expect("Error telegram responses should have descriptions")))
		}
	}
}
