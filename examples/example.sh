#!/bin/bash


# This is an example script for Bash. There is also a Fish example in this folder.
#
# Be aware that each example makes use of programs that you might not have
# installed like jq or the docker CLI. Even if you don't have the programs
# required by one of the commands, other commands may work for you.
#
# The "/hello" command only uses Bash built-in programs, so that one should always work.
#
# Run this script with a command like this:
#    tg-daemon --execute example.sh --commands-file commands.txt --bot-id <bot-id>


# We need a modern version of getopt to parse the commands from tg-daemon
getopt --test > /dev/null
if [[ $? -ne 4 ]]; then
	echo "The bash example script expects a modern version of getopt. Some commands will still work, remove this test condition at the top of the file to run it anyway."
	exit 1
fi


# Immediately halts execution if you don't have a required program installed
set -e


case $1 in
	# Greet the user with a truly stylish and elegant phrase
	'/hello')
		echo "Hello, World!"
		;;

	'/photo')
		echo "Sending a photo, this one is from Krystal Ng on Unsplash!"
		echo "//send"
		echo "//send-photo ./examples/krystal-ng-PrQqQVPzmlw-unsplash.jpg"
		;;

	'/file')
		echo "Sending a file!"
		echo "//send"
		echo "//send-file ./examples/file.txt"
		;;

	# Show error for unimplemented command
	'/math')
		echo "The /math command is only implemented for the Fish shell example."
		;;


	# Report the status of any running docker containers
	'/dps')
		echo "Active Docker Containers:"
		docker ps --format json | jq '[.State, .Names] | @tsv' -r
		;;


	# Get the air date and cover photo of the soonest upcoming anime from AniList
	'/anime')
		query='{"query":"query { AiringSchedule(notYetAired: true, sort: [TIME]) { airingAt, media { siteUrl, coverImage { large }, title { romaji } } } }"}'
		url='https://graphql.anilist.co'
		response=$(curl -H 'Content-Type: application/json' -d "$query" "$url")
		air_date=$(date -d "@$(echo $response | jq .data.AiringSchedule.airingAt)")
		title=$(echo $response | jq .data.AiringSchedule.media.title.romaji -r)
		cover_image_url=$(echo $response | jq .data.AiringSchedule.media.coverImage.large -r)
		site_url=$(echo $response | jq .data.AiringSchedule.media.siteUrl -r)
		curl $cover_image_url > /tmp/anime_cover_image.png

		echo "//send-photo /tmp/anime_cover_image.png"
		echo "$title"
		echo
		echo "$air_date"
		echo
		echo "$site_url"
		;;

	# Add a blue color overlay to an image using ImageMagick
	'/colorize')
		done=false

		while test $done = 'false'; do
			echo "I'm ready for a picture!"
			echo "//send"
			read -a response

			case "${response[0]}" in
				'/stop')
					done=true
					echo "Ok!"
					echo "//send"
					;;

				'//tg-photo')
					echo 'Oops, you uploaded a photo! Photos are compressed by telegram, for best results please send the picture as a file. (Or send /stop to stop.)'
					echo "//send"
					;;

				'//tg-document')
					done=true

					while test -z "$file_id" && test ${#response[@]} -gt 0; do
						if test "$response" = "--file-id"; then
							file_id=${response[1]}
						fi

						response=("${response[@]:1}")
					done

					echo "Receiving photo..."
					echo "//send"
					echo "//download-file $file_id"
					read _ignored file_path

					echo "Processing photo..."
					echo "//edit"
					magick $file_path -fill blue -colorize 50% $file_path

					echo "Uploading photo..."
					echo "//edit"
					echo "//send-photo $file_path"
					echo "//delete"
					;;

				*)
					echo 'I was expecting a file upload, please try again. (Or send "stop" to stop)'
					echo "//send"
					;;
			esac
		done
		;;

	'/ascii')
		echo "Send some text to convert to ASCII"
		echo "//send"

		read text
		set text $(string trim $text)

		echo "$text" | xxd -p -c 0 | sed 's/../& /g'
		;;


	'/dl')
		echo "Ready for URL! 😊"
		echo "//send"

		read url

		echo "Downloading $url. ✨"
		echo "//send"
		echo "//chat-action upload_document"

		temp_file=$(mktemp)
		curl "$url" > $temp_file
		echo "//send-file $temp_file"
		;;


	# Send the user the favicon and current player count of an active minecraft server
	# Try mc.hypixel.net
	'/mc')
		echo "Send a server address or tap one of the servers below!"
		echo "//inline-button callback mc.hypixel.net Hypixel"
		echo "//inline-button callback us.mineplex.com Mineplex"
		echo "//send"

		read -a response

		echo "//remove-inline-keyboard"
		echo "//chat-action typing"

		if test "//tg-callback" = "${response[0]}"; then
			server_url="${response[1]}"
		else
			server_url="${response[0]}"
		fi

		server_data=$(curl "https://api.mcsrvstat.us/2/$server_url")

		# Message the current and max player counts
		echo -e "Players: $(echo $server_data | jq .players.online)\nMax: $(echo $server_data | jq .players.max)"

		# Decode & send the favicon from the base64 in the response
		echo $server_data | jq .icon -r | cut -c 23- | base64 -d > /tmp/mcicon.png
		echo "//send-photo /tmp/mcicon.png"
		;;


	*)
		echo "I didn't understand that command"
		;;
esac
