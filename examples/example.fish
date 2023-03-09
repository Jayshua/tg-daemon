#!/bin/fish


# This is an example script for the Fish shell. There is also a Bash example in this folder.
#
# Be aware that each example makes use of programs that you might not have installed
# like jq or the docker CLI.
#
# Even if you don't have the programs required by one of the commands, other commands that
# you do have the programs for will work.
#
# The "/math" and "/hello" commands only use Fish built-in programs, so those should always work.
#
# Run this script with a command like this:
#    tg-daemon --execute example.fish --commands-file commands.txt --bot-id <bot-id>


# This will halt execution if you don't have a required program installed
function fish_command_not_found
	exit 1
end


switch $argv[1]
	# Greet the user with a truly stylish and elegant phrase
	case "/hello"
		echo "Hello, World!"


	# Do some math using fish's builtin math function
	case "/math"
		echo "Send some math!"
		echo "//send"
		read equation
		echo (math "$equation")


	# Get the air date and cover photo of the soonest upcoming anime from AniList
	case "/anime"
		set query '{"query":"query { AiringSchedule(notYetAired: true, sort: [TIME]) { airingAt, media { siteUrl, coverImage { large }, title { romaji } } } }"}'
		set url 'https://graphql.anilist.co'
		set response (curl -H 'Content-Type: application/json' -d "$query" "$url")
		set air_date (date -d "@$(echo $response | jq .data.AiringSchedule.airingAt)")
		set title (echo $response | jq .data.AiringSchedule.media.title.romaji -r)
		set cover_image_url (echo $response | jq .data.AiringSchedule.media.coverImage.large -r)
		set site_url (echo $response | jq .data.AiringSchedule.media.siteUrl -r)
		curl $cover_image_url > /tmp/anime_cover_image.png

		echo "//send-photo /tmp/anime_cover_image.png"
		echo "$title"
		echo
		echo "$air_date"
		echo
		echo "$site_url"


	# Add a blue color overlay to an image using ImageMagick
	case "/colorize"
		echo "I'm ready for a picture! (Be sure to upload it as a file, not as a photo.)"
		echo "//send"

		read --list response
		argparse -i 'file-id=' 'file-name=' 'mime-type=' -- $response

		switch $argv[1]
			case "//tg-document"
				echo "Receiving photo..."
				echo "//send"
				echo "//download-file $_flag_file_id"
				read _ignore file_path

				echo "Processing photo..."
				echo "//send"
				magick $file_path -fill blue -colorize 50% $file_path

				echo "//send-photo $file_path"
				echo "Done!"

			case "*"
				echo "I was expecting a file upload. Please resend the command to try again."
		end


	# Convert some text into ASCII hex using xxd
	case "/ascii"
		echo "Send some text to convert to ASCII"
		echo "//send"

		read text
		set text (string trim $text)

		echo "$text" | xxd -p -c 0 | sed 's/../& /g'


	# Download a file and send it back to the user
	case "/dl"
		echo "Ready for URL! 😊"
		echo "//send"

		read url
		set url (string trim $url)

		echo "Downloading $url. ✨"
		echo "//send"
		echo "//chat-action upload_document"

		set temp_file (mktemp)
		curl "$url" > $temp_file
		echo "//send-file $temp_file"


	# Send the user the favicon and current player count of an active minecraft server
	# Try mc.hypixel.net
	case "/mc"
		echo "Ready for the Server Address!"
		echo "//send"

		read server_address
		set server_address (string trim $server_address)
		set server_data (curl "https://api.mcsrvstat.us/2/$server_address")

		# Message the current and max player counts
		echo -e "Players: $(echo $server_data | jq .players.online)\nMax: $(echo $server_data | jq .players.max)"

		# Decode & send the favicon from the base64 in the response
		echo $server_data | jq .icon -r | cut -c 23- | base64 -d > /tmp/mcicon.png
		echo "//send-photo /tmp/mcicon.png"


	# Report the status of any running docker containers
	case "/dps"
		docker ps --format json | jq '[.State, .Names] | @tsv' -r


	case "*"
		echo "I didn't understand that command"
end