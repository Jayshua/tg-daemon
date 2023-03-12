use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::{TgClient, FILE_ID_ALPHABET};
use tracing::debug;
use serde_json::json;




// Types




/// Success/Failure type returned by all Telegram requests
///
/// Telegram always returns JSON of the form:
///    { "ok": bool, "result": ...whatever the endpoint returns..., "description": ...error... }
///
/// The result field is present iff ok is true.
/// The description field is present iff ok is false.
#[derive(Debug, serde::Deserialize)]
pub struct TelegramResponse<Data> {
	pub ok: bool,
	pub description: Option<String>,
	pub result: Option<Data>,
}

#[derive(Debug)]
pub struct TelegramError(pub String);

impl<Data> TelegramResponse<Data> {
	/// Convert a TelegramResponse into a Result that "?" can be easily used with
	pub fn to_result(self) -> Result<Data, TelegramError> {
		if self.ok {
			Ok(self.result.expect("Ok telegram responses should have results"))
		} else {
			Err(TelegramError(self.description.expect("Error telegram responses should have descriptions")))
		}
	}
}



/// Data returned from Telegram's getUpdates endpoint
#[derive(Debug, serde::Deserialize)]
pub struct UpdateResponse {
	pub update_id: u64,
	pub message: Option<Message>,
	pub callback_query: Option<CallbackQuery>,
}



/// Response when the user taps on an inline keyboard
#[derive(Debug, serde::Deserialize)]
pub struct CallbackQuery {
	pub id: String,
	pub data: String,
	pub message: Message,
}



/// A message and/or photo upload from the user
#[derive(Debug, serde::Deserialize)]
pub struct Message {
	pub message_id: u64,
	pub chat: Chat,
	pub text: Option<String>,
	pub document: Option<Document>,
	pub photo: Option<Vec<PhotoSize>>,
}



/// Every message is sent in a particular chat thread
#[derive(Debug, serde::Deserialize)]
pub struct Chat {
	pub id: u64,
}



/// A file upload
///
/// Despite the name this is used for any generic file.
/// It doesn't necessarily need to be a document - it could be an image file.
#[derive(Debug, serde::Deserialize)]
pub struct Document {
	pub file_id: String,

	/// These fields are renamed just as a reminder that they are
	/// provided by the end-user (not Telegram) and can't be trusted.
	#[serde(rename="file_name")]
	pub unsafe_file_name: Option<String>,

	/// These fields are renamed just as a reminder that they are
	/// provided by the end-user (not Telegram) and can't be trusted.
	#[serde(rename="mime_type")]
	pub unsafe_mime_type: Option<String>,
}



/// A particular variant of a photo uploaded by the user
///
/// Images can also be uploaded uncompressed, in which case they will
/// be Document structs rather than PhotoSize structs.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PhotoSize {
	pub file_id: String,
	pub width: u32,
	pub height: u32,
}



/// A temporary download link for a particular Document
/// Must be retrieved from Telegram separately from the Document struct itself.
#[derive(Debug, serde::Deserialize)]
pub struct File {
	pub file_path: Option<String>,
}




// Functions




/// Generic error returned by functions that don't need a more specific type
#[derive(Debug, derive_enum_from_into::EnumFrom)]
pub enum TgRequestError {
	TelegramError(TelegramError),
	Reqwest(reqwest::Error),
}



/// Send or update a message.
#[tracing::instrument(skip_all)]
pub async fn send_message(
	tg: TgClient,
	chat_id: u64,
	message_id: Option<u64>,
	text: Option<impl AsRef<str>>,
	keyboard: &[InlineKeyboardButton]
) -> Result<Message, TgRequestError> {
	// Ensure a message always has text
	assert!(message_id.is_some() || text.is_some());


	let mut body = serde_json::Map::new();

	body.insert("chat_id".to_string(), chat_id.into());

	if let Some(message_id) = message_id {
		body.insert("message_id".to_string(), message_id.into());
	}

	if let Some(text) = text {
		body.insert("text".to_string(), text.as_ref().into());
	}

	if keyboard.len() > 0 {
		let keyboard_json = keyboard
			.iter()
			.map(|keyboard| match &keyboard.variant {
				InlineKeyboardVariant::Url(url) =>
					json!({
						"text": keyboard.text,
						"url": url,
					}),

				InlineKeyboardVariant::Callback(callback) =>
					json!({
						"text": keyboard.text,
						"callback_data": callback,
					}),
			})
			.collect::<Vec<_>>();

		body.insert("reply_markup".to_string(), json!({ "inline_keyboard": vec![keyboard_json] }));
	}


	let url =
		if message_id.is_some() {
			if body.contains_key("text") {
				format!("{}/editMessageText", tg.bot_base())
			} else {
				format!("{}/editMessageReplyMarkup", tg.bot_base())
			}
		} else {
			format!("{}/sendMessage", tg.bot_base())
		};


	let message = tg.client
		.post(url)
		.json(&body)
		.send().await?
		.json::<TelegramResponse<Message>>().await?
		.to_result()?;


	Ok(message)
}

/// Passed to send_message to describe the inline buttons that a message should have
pub struct InlineKeyboardButton {
	pub text: String,
	pub variant: InlineKeyboardVariant,
}

/// An inline keyboard button can take the user to
/// a webpage or send a callback message back to the bot.
pub enum InlineKeyboardVariant {
	Url(String),
	Callback(String)
}



/// Delete a message
///
/// Telegram has a number of restrictions on what messages can be deleted.
/// Be sure to consult the documentation if you're not sure.
pub async fn delete_message(tg: TgClient, chat_id: u64, message_id: u64) -> Result<bool, TgRequestError> {
	let result = tg.client
		.post(format!("{}/deleteMessage", tg.bot_base()))
		.json(&json!({ "chat_id": chat_id, "message_id": message_id }))
		.send().await?
		.json::<TelegramResponse<bool>>().await?
		.to_result()?;

	Ok(result)
}



/// Use the /setMyCommands endpoint to setup the Menu button in the Telegram app
/// with the commands supported by the bot.
#[tracing::instrument(skip_all)]
pub async fn setup_commands(tg: TgClient, commands_path: &str) -> Result<(), SetupCommandsError> {
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

		commands.push(json!({
			"command": command,
			"description": description,
		}));
	}

	if commands.len() == 0 {
		return Err(SetupCommandsError::FileEmpty);
	}

	tg.client
		.post(format!("{}/setMyCommands", tg.bot_base()))
		.json(&json!({ "commands": commands }))
		.send().await?
		.json::<TelegramResponse<bool>>().await?
		.to_result()?;

	Ok(())
}

/// Errors possible when calling setup_commands
#[derive(Debug, derive_enum_from_into::EnumFrom)]
pub enum SetupCommandsError {
	FileIo(std::io::Error),
	ReqwestError(reqwest::Error),
	FileEmpty,
	InvalidCommandLine(usize),
	TelegramError(TelegramError),
}



/// Download a file from telegram into a temporary location on the file system
///
/// The OS will delete the file at some indeterminate point in the future.
/// Usually the next time the computer reboots, though some systems will delete sooner.
#[tracing::instrument(skip(tg))]
pub async fn download_file(tg: TgClient, chat_id: u64, file_id: &str) -> Result<std::path::PathBuf, DownloadFileError> {
	let file = tg.client
		.post(format!("{}/getFile", tg.bot_base()))
		.json(&json!({"file_id": file_id}))
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

/// Errors possible when calling the download_file function
#[derive(Debug, derive_enum_from_into::EnumFrom)]
pub enum DownloadFileError {
	Reqwest(reqwest::Error),
	FileIo(std::io::Error),
	TelegramError(TelegramError),
	FilePathMissing,
}



/// Set the bot's status
///
/// (The "typing...", "uploading file...", etc. status that shows up next to the bot's avatar.)
#[tracing::instrument(skip(tg))]
pub async fn send_chat_action(tg: TgClient, chat_id: u64, action: &str) -> Result<(), TgRequestError> {
	tg.client
		.post(format!("{}/sendChatAction", tg.bot_base()))
		.json(&json!({
			"chat_id": chat_id,
			"action": action,
		}))
		.send().await?
		.json::<TelegramResponse<serde_json::Value>>().await?
		.to_result()?;

	Ok(())
}



/// Send a file on the file system as a message
#[tracing::instrument(skip(tg))]
pub async fn send_file(tg: TgClient, chat_id: u64, file_path: impl AsRef<std::path::Path> + std::fmt::Debug) -> Result<Message, SendFileError> {
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

/// Send an image on the file system
///
/// Differs from send_file in that Telegram will compress photos sent with
/// this method but will not compress photos sent with send_file.
#[tracing::instrument(skip(tg))]
pub async fn send_photo(tg: TgClient, chat_id: u64, file_path: impl AsRef<std::path::Path> + std::fmt::Debug) -> Result<Message, SendFileError> {
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

/// Errors possible when calling the send_file or send_photo functions
#[derive(Debug, derive_enum_from_into::EnumFrom)]
pub enum SendFileError {
	FileIo(std::io::Error),
	Reqwest(reqwest::Error),
	Telegram(TelegramError),
}
