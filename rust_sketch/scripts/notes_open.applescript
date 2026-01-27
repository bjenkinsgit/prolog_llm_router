(*
@tool notes_open
@version 1.0
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
            return "OK: Opened note: " & (name of targetNote)
        end tell

    on error errMsg
        return "ERROR: " & errMsg
    end try
end run
