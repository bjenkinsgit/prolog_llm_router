% =========================
% Tool Router KB (router.pl)
% =========================

% --- tool capabilities ---
provides(search_notes, search(notes)).
provides(search_files, search(files)).
provides(get_weather, info(weather)).
provides(draft_email, compose(email)).
provides(create_todo, create(todo)).
provides(summarize, transform(text_summary)).

% --- safety / policy examples ---
forbidden(draft_email, context(untrusted_recipient)).
forbidden(search_files, context(scope(system))).

% --- routing rules ---
% route(Intent, Entities, Constraints, Tool, Args).

route(summarize, Entities, Constraints, search_notes, [query(Q)]) :-
    preferred_source(Constraints, notes),
    topic_query(Entities, Q).

route(summarize, Entities, _Constraints, search_files, [query(Q), scope(user)]) :-
    topic_query(Entities, Q).

route(find, Entities, Constraints, search_notes, [query(Q)]) :-
    preferred_source(Constraints, notes),
    topic_query(Entities, Q).

route(find, Entities, _Constraints, search_files, [query(Q), scope(user)]) :-
    topic_query(Entities, Q).

route(weather, Entities, _Constraints, get_weather, [location(L), date(D)]) :-
    entity(Entities, location, L),
    entity(Entities, date, D).

route(draft, Entities, _Constraints, draft_email, [to(To), subject(S), body("")]) :-
    entity(Entities, recipient, To),
    entity(Entities, topic, S).

route(remind, Entities, _Constraints, create_todo, [title(T), due(D)]) :-
    entity(Entities, topic, T),
    entity(Entities, date, D).

% --- ask follow-ups when missing key info ---
need_info(weather, Entities, "What location should I use for the weather?") :-
    \+ entity(Entities, location, _).

need_info(remind, Entities, "When is this due?") :-
    \+ entity(Entities, date, _).

need_info(draft, Entities, "Who should I email?") :-
    \+ entity(Entities, recipient, _).

% --- helpers ---
preferred_source(Constraints, notes) :-
    constraint(Constraints, source_preference, notes).

preferred_source(Constraints, notes) :-
    constraint(Constraints, source_preference, either).

topic_query(Entities, Q) :-
    ( entity(Entities, topic, Q) -> true
    ; entity(Entities, query, Q) ).

entity(Entities, Key, Value) :-
    get_dict(Key, Entities, Value).

constraint(Constraints, Key, Value) :-
    get_dict(Key, Constraints, Value).
