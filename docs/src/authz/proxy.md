# Proxy

Dispatcher performs proxying of authz requests.

If authz object starts with `["room", ROOM_ID, ..]` and dispatcher manages to find a webinar/p2p/chat corresponding to the room,
the authz object will be altered.

Otherwise, the request will be proxied as is (only if the audience param is a valid audience for authorization)

Modifications besides `["rooms", ROOM_ID]` override are temporary.

The rules for this modification are[^1]:

* if request comes from `event`:

Object                                          | Action      | New object                                  | New action
-------------------------------                 | ----------- | ------------------                          | ------------
["rooms", ROOM_ID, "agents"]                    | `list`      | ["classrooms", ID]                          | `read`
["rooms", ROOM_ID, "events"]                    | `list`      | ["classrooms", ID]                          | `read`
["rooms", ROOM_ID, "events"]                    | `subscribe` | ["classrooms", ID]                          | `read`
["rooms", ROOM_ID, "events", "draw_lock", ..]   | `create`    | ["classrooms", ID, "events", "draw", ..]    | `create`
["rooms", ROOM_ID, ..]                          | *           | ["classrooms", ID]                          | no change[^2]
\*                                              | *           | no change                                   | no change[^3]

* if request comes from `conference`:

Object                          | Action      | New object         | New action
------------------------------- | ----------- | ------------------ | ------------
["rooms", ROOM_ID, "agents"]    | `list`      | ["classrooms", ID] | `read`
["rooms", ROOM_ID, "rtcs"]      | `list`      | ["classrooms", ID] | `read`
["rooms", ROOM_ID, "events"]    | `subscribe` | ["classrooms", ID] | `read`
["rooms", ROOM_ID, ..]          | *           | ["classrooms", ID] | no change[^2]
\*                              | *           | no change          | no change[^3]

* if request comes from `storage (v2)`:

Object                          | Action      | New object                                      | New action
------------------------------- | ----------- | ----------------------------------------------- | ------------
["sets", "origin" <> _]         | *           | ["classrooms", ID]                              | upload
["sets", "ms" <> _]             | *           | ["classrooms", ID]                              | download
["sets", "meta" <> _]           | read        | ["classrooms", ID]                              | read
["sets", "hls" <> _]            | read        | ["classrooms", ID]                              | read
["sets", "content" <> _]        | create      | ["classrooms", ID, content]                     | update
["sets", "content" <> _]        | delete      | ["classrooms", ID, content]                     | update
["sets", "content" <> _]        | read        | ["classrooms", ID]                              | read
["sets", SET]                   | *           | ["classrooms", ID, "sets", BUCKET_PREFIX][^4]   | no change[^2]
\*                              | *           | no change                                       | no change[^3]

[^1]: `["rooms", ROOM_ID, ..]` means an array containing at least 2 elements, you can read `, ..` as "0 or more elements".

[^2]: If a class corresponding to the room is found action is left as is, only object is altered.

[^3]: if a class corresponding to the room or set is not found everything stays the same.

[^4]: set has format like `#{bucket_prefix.audience}::#{set_id}`
