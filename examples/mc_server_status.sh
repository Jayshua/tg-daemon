#!/bin/sh

# This example returns the favicon and status of a minecraft server to the client
# using https://mcsrvstat.us/, jq, and base64

if [ $1 = '/mc' ]; then
	output=$(curl "https://api.mcsrvstat.us/2/$2")

	# Message the current and max player counts
	echo -e "Players: $(echo $output | jq .players.online)\nMax: $(echo $output | jq .players.max)"

	# Decode & send the favicon from the base64 in the response
	echo $output | jq .icon -r | cut -c 23- | base64 -d > /tmp/mcicon.png
	echo "/send_photo /tmp/mcicon.png"

else
	echo "I didn't understand that"

fi