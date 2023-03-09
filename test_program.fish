#!/bin/fish

switch $argv[1]
	case "/host"
		echo "Please send a file"
		echo "//send"

		read response

		if string match -q -r "^//tg-document" $response
			echo "Response: $response"
			argparse 'file-id=' 'file-name=' 'mime-type=' -- $response
			echo "//chat-action upload_document"
			echo "Processing file: $_flag_file_id"
			echo "//send"
			echo "//download-file $_flag_file_id"
			read command file_path
			echo "Downloaded! $command $file_path"

			cp $file_path ~/public/files/$file_name

		else if string match -r "^//tg-photo" $response
			echo "Please send as a file."
		else
			echo "Sorry, I expected a file. Please try sending /host again."
		end

	case '*'
		echo "Unknown command"
end
