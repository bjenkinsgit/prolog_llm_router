% ====================================
% router_standard.pl — Deterministic Tool Router (Standard Prolog)
% ====================================
% This version uses association lists instead of SWI-Prolog dicts,
% making it compatible with Scryer Prolog and other ISO Prolog implementations.
%
% Data format:
%   Entities and Constraints are association lists: [key-value, key2-value2, ...]
%   Args are also association lists: [query-"AI", scope-user]

% Tool capability facts (expand later)
provides(search_notes, search(notes)).
provides(search_files, search(files)).
provides(get_weather, info(weather)).
provides(draft_email, compose(email)).
provides(create_todo, create(todo)).

% -----------------------------
% Routing rules
% route(Intent, Entities, Constraints, Tool, Args).
% Entities and Constraints are assoc lists, e.g. [topic-"x", location-"y"].
% -----------------------------

% Summarize / Find: prefer notes unless user prefers files explicitly
route(summarize, E, C, search_notes, [query-Q]) :-
    preferred_source(C, notes),
    topic_query(E, Q).

route(summarize, E, _C, search_files, [query-Q, scope-user]) :-
    topic_query(E, Q).

route(find, E, C, search_notes, [query-Q]) :-
    preferred_source(C, notes),
    topic_query(E, Q).

route(find, E, _C, search_files, [query-Q, scope-user]) :-
    topic_query(E, Q).

% Weather requires location/date
route(weather, E, _C, get_weather, [location-L, date-D]) :-
    must_get(E, location, L),
    must_get(E, date, D).

% Draft requires recipient
route(draft, E, _C, draft_email, [to-To, subject-S, body-""]) :-
    must_get(E, recipient, To),
    get_with_default(E, topic, "(no subject)", S).

% Remind creates a todo
route(remind, E, _C, create_todo, [title-T, due-D, priority-P]) :-
    must_get(E, topic, T),
    must_get(E, date, D),
    get_with_default(E, priority, "normal", P).

% -----------------------------
% Follow-up questions
% need_info(Intent, Entities, Question).
% -----------------------------
need_info(weather, E, "What location should I use?") :-
    \+ get_key(E, location, _).

need_info(weather, E, "What date should I use? (e.g., today, tomorrow)") :-
    \+ get_key(E, date, _).

need_info(remind, E, "When is this due?") :-
    \+ get_key(E, date, _).

need_info(draft, E, "Who should I email?") :-
    \+ get_key(E, recipient, _).

need_info(summarize, E, "What topic should I summarize?") :-
    \+ get_key(E, topic, _),
    \+ get_key(E, query, _).

need_info(find, E, "What should I search for?") :-
    \+ get_key(E, topic, _),
    \+ get_key(E, query, _).

% -----------------------------
% Helpers (standard Prolog compatible)
% -----------------------------

% Get a value from an association list
% get_key(AssocList, Key, Value)
get_key([Key-Value|_], Key, Value) :- !.
get_key([_|Rest], Key, Value) :- get_key(Rest, Key, Value).

% Check if a key exists
has_key(List, Key) :- get_key(List, Key, _).

% Get with default value
get_with_default(List, Key, _Default, Value) :-
    get_key(List, Key, Value), !.
get_with_default(_List, _Key, Default, Default).

% Must get - throws if key is missing
must_get(List, Key, Value) :-
    ( get_key(List, Key, Value) -> true
    ; throw(error(missing_required(Key), context(must_get/3, Key)))
    ).

% Source preference logic
preferred_source(C, notes) :-
    get_with_default(C, source_preference, either, Pref),
    (Pref = notes ; Pref = either).

% Get topic or query
topic_query(E, Q) :-
    ( get_key(E, topic, Q)
    ; get_key(E, query, Q)
    ).
