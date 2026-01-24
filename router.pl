% ====================================
% router.pl — Deterministic Tool Router
% ====================================

% Tool capability facts (expand later)
provides(search_notes, search(notes)).
provides(search_files, search(files)).
provides(get_weather, info(weather)).
provides(draft_email, compose(email)).
provides(create_todo, create(todo)).

% -----------------------------
% Routing rules
% route(Intent, Entities, Constraints, Tool, Args).
% Entities and Constraints are SWI dicts, e.g. _{topic:"x", location:"y"}.
% -----------------------------

% Summarize / Find: prefer notes unless user prefers files explicitly
route(summarize, E, C, search_notes, _{query:Q}) :-
    preferred_source(C, notes),
    topic_query(E, Q).

route(summarize, E, _C, search_files, _{query:Q, scope:user}) :-
    topic_query(E, Q).

route(find, E, C, search_notes, _{query:Q}) :-
    preferred_source(C, notes),
    topic_query(E, Q).

route(find, E, _C, search_files, _{query:Q, scope:user}) :-
    topic_query(E, Q).

% Weather requires location/date
route(weather, E, _C, get_weather, _{location:L, date:D}) :-
    must_get(E, location, L),
    must_get(E, date, D).

% Draft requires recipient
route(draft, E, _C, draft_email, _{to:To, subject:S, body:""}) :-
    must_get(E, recipient, To),
    get_with_default(E, topic, "(no subject)", S).

% Remind creates a todo
route(remind, E, _C, create_todo, _{title:T, due:D, priority:P}) :-
    must_get(E, topic, T),
    must_get(E, date, D),
    get_with_default(E, priority, "normal", P).

% -----------------------------
% Follow-up questions
% need_info(Intent, Entities, Question).
% -----------------------------
need_info(weather, E, "What location should I use?") :-
    \+ get_dict(location, E, _).

need_info(weather, E, "What date should I use? (e.g., today, tomorrow)") :-
    \+ get_dict(date, E, _).

need_info(remind, E, "When is this due?") :-
    \+ get_dict(date, E, _).

need_info(draft, E, "Who should I email?") :-
    \+ get_dict(recipient, E, _).

need_info(summarize, E, "What topic should I summarize?") :-
    \+ get_dict(topic, E, _),
    \+ get_dict(query, E, _).

need_info(find, E, "What should I search for?") :-
    \+ get_dict(topic, E, _),
    \+ get_dict(query, E, _).

% -----------------------------
% Helpers
% -----------------------------

preferred_source(C, notes) :-
    get_with_default(C, source_preference, either, Pref),
    (Pref = notes ; Pref = either).

topic_query(E, Q) :-
    ( get_dict(topic, E, Q)
    ; get_dict(query, E, Q)
    ).

must_get(Dict, Key, Value) :-
    ( get_dict(Key, Dict, Value) -> true
    ; throw(error(missing_required(Key), _))
    ).

get_with_default(Dict, Key, Default, Value) :-
    ( get_dict(Key, Dict, Value) -> true ; Value = Default ).
