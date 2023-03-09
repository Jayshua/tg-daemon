#!/bin/sh

# This is one possible way to handle multiple commands in a single script.
# There are probably other ways, but this one is nice and simple.

# Message a truly classic phrase
if [ $1 = '/hello' ]; then
	echo "Hello, World!"

# Report the size of the /tmp directory
else if [ $1 = '/tmpsize' ]; then
	tmp_dir_size=$(du -hs /tmp | awk '{ print $1 }')

fi
