# Webinars authorization objects

Object                                                           | Action  | Description
---------------------------------------------------------------- | ------- | ------------
["webinars"]                                                     | create  | Tenant [creates](/webinars/api.md#create-webinar) a webinar
["webinars"]                                                     | convert | Tenant [converts](/webinars/api.md#update-webinar) already existings rooms into a webinar
["webinars", WEBINAR_ID]                                         | update  | Tenant or user [updates](/webinars/api.md#update-webinar) a webinar [^1]
["webinars", WEBINAR_ID]                                         | read    | User [reads](/webinars/api.md#read-webinar) the webinar state
["webinars", WEBINAR_ID, "events", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new event [^2] in the webinar
["webinars", WEBINAR_ID, "claims", TYPE, "authors", ACCOUNT_ID]  | create  | User creates a new claim [^2] in the webinar
["webinars", WEBINAR_ID, ATTRIBUTE, TYPE, "authors", ACCOUNT_ID] | create  | User alter an event [^2] somehow
["webinars", WEBINAR_ID, "rtcs"]                                 | create  | User creates an RTC
["webinars", WEBINAR_ID, "rtcs", RTC_ID]                         | update  | User streams in an RTC
["webinars", WEBINAR_ID, "sets", "content"]                      | read    | User reads a file
["webinars", WEBINAR_ID, "sets", "content"]                      | create  | User uploads a file
["webinars", WEBINAR_ID, "sets", "content"]                      | delete  | User deletes a file
["webinars", WEBINAR_ID, "sets", "origin"]                       | read    | User reads an original recording
["webinars", WEBINAR_ID, "sets", "origin"]                       | create  | User uploads an original recording
["webinars", WEBINAR_ID, "sets", "ms"]                           | read    | User reads a modified recording
["webinars", WEBINAR_ID, "sets", "hls"]                          | read    | User reads an hls fragment (think watching recording with a web player)
["webinars", WEBINAR_ID, "sets", "meta"]                         | read    | User reads a recording metadata (thumbnails, etc)

[^1]: This is both time updates for a tenant, stop for a running webinar and media-editor access

[^2]: Types, claims and attributes are documented [separately](./events.md)
