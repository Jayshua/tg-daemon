#!/bin/fish

echo "Args: $argv"
echo "Downloading File"
echo "//send"
echo "//chat-action upload_document"
echo "//download-file BQACAgEAAxkBAAICwmQFgVeRZ4MtXUziqtAaoHRU9iZ5AAJTAwACuxkwRDTQ8qigeqjPLgQ"
read unused file_path
echo "Unused: $unused"
echo "File path: $file_path"
cp $file_path .
