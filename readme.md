# tg-daemon

A daemon that will connect to Telegram as a bot and forward chat messages to another program over stdin/stdout. Kind of like classic CGI.


## Getting Started

1. Get the code  
	Clone the repo and run `cargo build --release`, then copy `target/release/tg-daemon` to the bin directory your distribution provides for manually installed executables. (Usually /usr/local/bin)

2. Get a Telegram Bot ID  
	Message `/newbot` to the BotFather in the Telegram app. BotFather will walk you through the setup process.  
	https://telegram.me/BotFather

3. Write an executable for tg-daemon to run when your bot receives a message, or copy one of the example scripts from the examples directory. See the "How to write a handler executable" section below.

4. Run tg-daemon with a command like: `tg-daemon --executable script.sh --bot-id slkfjlaksdfjlskfjlskdf`, or run `tg-daemon --help` for more options.

5. (Optional) Lockdown tg-daemon to only respond to your personal chat messages (Optional, but **strongly** recommended. See the Caveats section below.)  
	You can find your chat_id by setting the LOG_LEVEL environment variable to "INFO" and looking at tg-daemon's logs, or echoing the CHAT_ID environment variable in your handler script.

6. (Optional) Run tg-daemon as a daemon/service  
	You'll have to consult your Linux distribution's documentation for this one.  
	In Void Linux you would create a service directory and link it with `ln` in the /var/service directory.


## How to write a handler script

Handlers just need to be an executable that can read/write to stdin/stdout. I usually use Fish or Bash, but any language will work. I'll use Bash in these examples. tg-daemon will spawn an instance of your script when a message is received from telegram. Subsequent messages will be forwarded over stdin until your script terminates.

This is one of the simplest possible handler scripts:

```bash
#!/bin/bash

# Run this with: tg-daemon --execute ./script.sh --bot-id <bot-id>
# You might find more detailed logging from tg-daemon useful when debugging your script:
# LOG_LEVEL=INFO tg-daemon --execute ./script.sh --bot-id <bot-id>

echo "Hello, World!"
```

That script will respond to every message with the text "Hello, World!". A truly classic and stylish greeting.

You can send multiple messages with the `//send` command:

```bash
# Will send two separate message bubbles
echo "First Message"
echo "//send"
echo "Second Message"
echo "//send"

# Will send one message bubble with two lines
echo "First Line"
echo "Second Line"
echo "//send"

# The last //send is optional, any echoed but unset text after the script terminates will be sent automatically.
echo "This message will be sent after the script terminates."
```

You can edit the last sent message with the `//edit` command:

```bash
echo "Hello!"
echo "//send" # Don't forget to send before sleep - pending message text won't be sent until the script terminates.

sleep 5

echo "Goodbye!"
echo "//edit"
```

You can delete the last sent message with the `//delete` command:

```bash
echo "Hello!"
echo "//send"

sleep 5

echo "//delete"
```

You can set the bot's status (the "typing..." or "uploading..." text that appears for a few seconds next to the avatar) with the `//chat-action` command:

```bash
echo "Hello"
echo "//send"
echo "//chat-action typing"
sleep 4
echo "World!"
```

Command parsing can be suppressed with `//heredoc` until an arbitrary terminator is echoed:

```bash
echo "Message 42"
echo "//send"
echo "//delete" > /tmp/command.txt
echo "//heredoc END_HEREDOC"
cat /tmp/command.txt # This would delete Message 42 if not inside a heredoc. Instead it sends the literal text "//delete" as a message.
echo "END_HEREDOC"
```

You can send a file with the `//send-file` command:

```bash
# Remember: the working directory will be the directory tg-daemon is run in, not the directory of the script.
echo "content for the example text file" > ./asdf.txt
echo "//send-file ./asdf.txt"
```

You can send a photo with the `//send-photo` command:

```bash
# Photos are automatically compressed into multiple sizes by Telegram to shorten download times
echo "//send-photo /path/to/photo.jpg"
```

tg-daemon will provide the user's message as the first argument when spawning the script process, similar to calling the script from the terminal:

```bash
case $1 in
	# By convention, Telegram bots respond to "/command" messages
	'/hello')
		echo "Hello, World!"
		;;

	# But if the user doesn't start their message with a "/" it still works
	'goodbye')
		echo "Goodbye, World!"
		;;

	*)
		echo "Unknown Command"
		;;
esac
```

You can ask the user for more info by reading from stdin:

```bash
echo "What is your name?"
echo "//send"
read response
echo "Hello $response!"
```

tg-daemon will send you commands, which I call "callbacks", to let you know when interesting things happen. Callbacks always start with `//tg-`.

```bash
echo "Please send me a photo"
echo "//send"

read -a response # The -a flag splits the read text into an array on whitespace (Sort of. Consult the bash documentation for more info.) 

if [[ "${response[0]}" = "//tg-photo" ]]; then
	file_id=${response[1]}
	echo "Photo received! The telegram file id is: $file_id"
else
	echo "Oops, I expected a photo!"
fi
```

You can download files uploaded to telegram with the `//download-file` command. tg-daemon will download the file from Telegram and save it in a temporary location (probably in the '/tmp' directory), notifying you when the download is complete with the `//tg-file-download` callback.

```bash
echo "//download-file $file_id"

read -a response

if [[ "${response[0]}" = "//tg-file-download" ]]; then
	echo "Photo saved to the temporary path: ${response[1]}"
fi
```

You can give the user some buttons to tap with the `//inline-button` command:

```bash
echo "How would you like to be greeted?"

# These buttons will cause tg-daemon to send //tg-callback when the user taps them
echo "//inline-button callback standard-greeting With Standard Greeting"
echo "//inline-button callback jedi-greeting With A Jedi Greeting"

# This button will cause the user's telegram client to open their web browser when tapped
echo "//inline-button url https://google.com/search?q=what+is+a+greeting What is a Greeting?"

# Buttons are queued up into a list and attached to the next sent message
echo "//send"
```

Listen for taps on `//inline-button callback` buttons with the `//tg-callback` callback:

```bash
read -a response

if [[ ${response[0]} = '//tg-callback' ]]; then
	case "${response[1]}" in
		'standard-greeting')
			echo "Hello, World!"
			;;

		'jedi-greeting')
			echo "Hello there!"
			;;
	esac
fi
```

`//inline-button url` buttons do not generate `//tg-callback` when tapped.

The inline keyboard attached to the most recent message can be deleted with `//remove-inline-keyboard`:

```bash
echo "Tap a button!"
echo "//inline-button ..."
echo "//inline-button ..."
echo "//send"

read -a response

echo "//remove-inline-keyboard" # You probably want to make sure ${response} is a //tg-callback first.
```

A separate instance of the handler script is spawned for each chat the bot is part of. You can access the unique id of the chat (provided by telegram) in the CHAT_ID environment variable:

```bash
echo "$CHAT_ID"
```

You can restrict tg-daemon to only accept messages from authorized chats with the --chat-id flag, which can be used multiple times:

```bash
tg-daemon --execute ./example.sh --bot-id <bot-id> --chat-id 1231231234 --chat-id 4564564567
```

tg-daemon will send "Unauthorized" to unauthorized chats, and will not spawn an instance of the handler script.

That's all the basics! There are a few more details you can find in the reference documentation below. Checkout the examples directory for some more complex handler scripts.



## Docs

### CLI Params

These parameters are provided when running tg-daemon. Only `--execute` and `--bot-id` are required.

**--execute &lt;path-to-executable&gt;**  
Path to the executable to spawn and send messages to

**--bot-id &lt;bot-id&gt;**  
ID of the telegram bot to listen for messages to.
You can get one of these from the BotFather (https://telegram.me/BotFather)

**--chat-id &lt;chat-id&gt;**  
ID(s) of the authorized chats. tg-daemon will ignore messages sent from unauthorized chats.

All messages will be handled if `--chat-id` is missing.

You can use `--chat-id` as many times as you like.

**--commands-file &lt;file-path&gt;**  
Tell Telegram what commands the bot supports.

Path should point to a file containing a command-description space separated pair on each line.

Telegram will use the list to generate a "Menu" button in the app. Run tg-daemon with "--help"
for more detail, or look in the examples folder for an example of the expected file format.

**--send-handler-errors**  
Send details of handler process crashes to the Telegram chat in
addition to the normal "Fatal Server Error" message.

**--tg-api-url**  
URL to access the Telegram API at.
I'm not sure why you would want to change this. Maybe if you're running a development
version of Telegram's bot server?

**--pipe-first-message**  
When spawning a new handler process, send the first message to stdin

By default the first message will be sent via spawning args, as if your executable was
run from the command line. (The args accessible with $1, $2, etc. in bash.)

With this flag, the first message will instead be sent to stdin just like subsequent
messages are.




### Commands

Commands are sent from the handler process to tg-daemon to instruct it to do things like send messages or upload photos.

**//send**  
Send all buffered text as a single message

**//edit**  
Same as `//send`, but replaces the last sent message rather than sending a new one.

**//delete**  
Delete the last sent message

**//inline-button &lt;url|callback&gt; &lt;url_string|callback_data&gt; &lt;button_text&gt;**  
Queue an inline button to be sent with the next message.

- kind
	- `url` will cause the button to open the user's web browser to the `url_string` when tapped
	- `callback` will cause `//tg-callback <callback_data>` to be sent over stdin when the user taps the button

```
//inline-button callback clicked-api-data All API Data Listings
//inline-button url https://www.duckduckgo.com Open a Safe Search Engine
```

**//remove-inline-keyboard**  
Remove the inline keyboard attached to the most recent message.
Mainly exists for clarity - is equivalent to calling `//edit` without calling `//inline-button` or echoing any message text.


**//download-file &lt;file_id&gt;**  
Download a file from the id given by `//tg-document` or `//tg-photo`, saving it to a temporary file whose path will be sent back over stdin with `//tg-file-download`.

**//chat-action &lt;action&gt;**  
Set the bot's chat action status. This is the "typing" or "uploading file" status that appears next to the bot's avatar.

&lt;action&gt; can be one of:
- typing
- upload_photo
- record_video
- upload_video
- record_voice
- upload_voice
- upload_document
- choose_sticker
- find_location
- record_video_note
- upload_video_note


**//send-photo &lt;file_path&gt;**  
Send the photo at the given file path as an image.
Telegram automatically compresses photos for best performance. To avoid this, use `//send-file` instead.
If the file is inaccessable for some reason, the entire handler process will be terminated.


**//send-file &lt;file_path&gt;**  
Send the file at the given path.
If the file is inaccessable for some reason, the entire handler process will be terminated.


**//heredoc &lt;terminator&gt;**  
Ignore any subsiquent commands, treating them as plain text, until the given &lt;terminator&gt; is
found at the start of a line without any whitespace before it.



### Callbacks

Callbacks are sent from tg-daemon to the handler process to inform it of events.
Not to be confused with callback_data, which comes from Telegram when the user taps an inline keyboard button.

**//tg-document --file-id &lt;file_id&gt; [--file-name &lt;file_name&gt;] [--mime-type &lt;mime_type&gt;]**  
The user uploaded a file. Use `//download-file` to retrieve it.

If the user's telegram client provided a file name or mime type, those will be available as well.
Note that this is user provided data. Although tg-daemon will parse them to ensure they don't contain
any dangerous characters (like spaces, since this is a space separated protocol), there's no guarantee
the mime type is correct or the file name is trustworthy.

**//tg-photo [&lt;file_id&gt; &lt;width&gt; &lt;height&gt;...]**  
The user uploaded a photo.

Telegram automatically compresses photos into multiple sizes for best performance. Each size Telegram provides
will be included in the `//tg-photo` callback as a space separated id-width-height triple.

**//tg-file-download &lt;file_path&gt;**  
The file requested with the `//download-file` command has been downloaded to the given path.
This will be a temporary file, probably in the `/tmp` directory, so be sure to move it somewhere if you want to keep it.

**//tg-callback &lt;callback_data&gt;**  
The user tapped an inline button defined with the `//inline-button callback <callback_data>` command.

**//tg-unknown**  
Telegram sent tg-daemon an update message that it didn't understand. You can probably just ignore this message.



## Caveats

**tg-daemon SHOULD NOT BE USED TO WRITE A PRODUCTION TELEGRAM BOT**

tg-daemon was designed to run my personal telegram bot that I use to do random stuff on my hobby server. While the handler executable tg-daemon spawns doesn't *have* to be insecure, it is very, very easy to write it in an insecure way. For example, interpolating user input from a telegram message into a command like this: `curl https://example.com/$2` could easily be turned into a remote code execution vulnerability.

Also, anyone can find your telegram bot by searching in the telegram app. Then they can send it commands.

I *strongly* recommend only using this daemon for personal scripts, and using the `--chat-id` CLI flag to lockdown to just your own chat.

All of that said, there are a few features to prevent obvious issues:

- Multiple leading slashes in messages are collapsed into a single slash before forwarding to the handler script to prevent the user from impersonating tg-daemon by sending "//tg-file-download" or similar. 

- User-provided file names reported with `//tg-document` are sanitized to just these characters: [A-Za-z0-9_.].

- User-provided mime-types reported with `//tg-document` are parsed and dropped if not recognized.
