(*
@tool notes_open
@version 1.1
@input note_id:string (argv 1)
@output Opens the note in Notes.app, returns success message or error
@errors Outputs "ERROR: message" on failure
*)

on run argv
    -- Validate arguments
    if (count of argv) < 1 then
        return "ERROR: Missing note ID argument"
    end if

    set noteId to item 1 of argv

    try
        tell application "Notes"
            activate
            set targetNote to missing value

            repeat with n in every note
                if (id of n) = noteId then
                    set targetNote to n
                    exit repeat
                end if
            end repeat

            if targetNote is missing value then
                return "ERROR: Note not found with ID: " & noteId
            end if

            show targetNote
            set noteName to name of targetNote
        end tell

        -- Press Return to open the selected note in the editor
        delay 0.3
        tell application "System Events"
            tell process "Notes"
                set frontmost to true
                delay 0.2
                key code 36 -- Return key
            end tell
        end tell

        return "OK: Opened note: " & noteName

    on error errMsg
        return "ERROR: " & errMsg
    end try
end run
