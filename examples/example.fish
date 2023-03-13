#!/bin/fish


# This is an example script for the Fish shell. There is also a Bash example in this folder.
#
# Be aware that each example makes use of programs that you might not have
# installed like jq or the docker CLI. Even if you don't have the programs
# required by one of the commands, other commands may work for you.
#
# The "/math" and "/hello" commands only use Fish built-in programs, so those should always work.
#
# Run this script with a command like this:
#    tg-daemon --execute example.fish --commands-file commands.txt --bot-id <bot-id>


# Immediately halts execution if you don't have a required program installed
function fish_command_not_found
	exit 1
end


switch $argv[1]
	# Greet the user with a truly stylish and elegant phrase
	case "/hello"
		echo "Hello, World!"


	# Attach a photo
	case "/photo"
		echo "Sending a photo, this one is from Krystal Ng on Unsplash!"
		echo "//send"
		echo "//send-photo ./examples/krystal-ng-PrQqQVPzmlw-unsplash.jpg"


	# Attach a file
	case "/file"
		echo "Sending a file!"
		echo "//send"
		echo "//send-file ./examples/file.txt"


	# Do some math using fish's builtin math function
	case "/math"
		echo "Send some math!"
		echo "//send"
		read equation
		echo (math "$equation")


	# Report the status of any running docker containers
	case "/dps"
		echo "Active Docker Containers:"
		docker ps --format json | jq '[.State, .Names] | @tsv' -r


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
		set done false

		while test $done = 'false'
			echo "I'm ready for a picture!"
			echo "//send"
			read --list response
			argparse -i 'file-id=' 'file-name=' 'mime-type=' -- $response

			switch $argv[1]
				case '//tg-document'
					set done true

					echo "Receiving photo..."
					echo "//send"
					echo "//download-file $_flag_file_id"
					read _ignore file_path

					echo "Processing photo..."
					echo "//edit"
					magick $file_path -fill blue -colorize 50% $file_path

					echo "Uploading photo..."
					echo "//edit"
					echo "//send-photo $file_path"
					echo "//delete"

				case '/stop'
					set done true
					echo "Ok!"
					echo "//send"

				case '//tg-photo'
					echo 'Oops, you uploaded a photo! Photos are compressed by telegram, for best results please send the picture as a file. (Or send /stop to stop.)'
					echo "//send"

				case '*'
					echo 'I was expecting a file upload, please try again. (Or send /stop to stop)'
					echo "//send"
			end
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

		read -a url # Reading the url into an array with -a strips the trailing

		echo "Downloading $url. ✨"
		echo "//send"
		echo "//chat-action upload_document"

		set temp_file (mktemp)
		curl $url > $temp_file
		echo "//send-file $temp_file"


	# Send the user the favicon and current player count of an active minecraft server
	# Try mc.hypixel.net
	case "/mc"
		echo "Send a server address or tap one of the servers below!"
		echo "//inline-button callback mc.hypixel.net Hypixel"
		echo "//inline-button callback us.mineplex.com Mineplex"
		echo "//send"

		read --array response

		echo "//remove-inline-keyboard"
		echo "//chat-action typing"

		if test "//tg-callback" = "$response[1]"
			set server_url "$response[2]"
		else
			set server_url "$response[1]"
		end

		set server_data (curl "https://api.mcsrvstat.us/2/$server_url")

		# Message the current and max player counts
		echo -e "Players: $(echo $server_data | jq .players.online)\nMax: $(echo $server_data | jq .players.max)"

		# Decode & send the favicon from the base64 in the response
		echo $server_data | jq .icon -r | cut -c 23- | base64 -d > /tmp/mcicon.png
		echo "//send-photo /tmp/mcicon.png"


	case "*"
		echo "I didn't understand that command"
end