# Webinars authorization objects

Object                                                           | Action   | Description
---------------------------------------------------------------- | -------- | ------------
["classrooms"]                                                     | create   | Tenant [creates](/webinars/api.md#create-webinar) a webinar
["classrooms"]                                                     | convert  | Tenant [converts](/webinars/api.md#update-webinar) already existings rooms into a webinar
["classrooms", WEBINAR_ID]                                         | update   | Tenant or user [updates](/webinars/api.md#update-webinar) a webinar [^1]
["classrooms", WEBINAR_ID]                                         | read     | User [reads](/webinars/api.md#read-webinar) the webinar state
["classrooms", WEBINAR_ID, "events", TYPE, "authors", ACCOUNT_ID]  | create   | User creates a new event [^2] in the webinar
["classrooms", WEBINAR_ID, "claims", TYPE, "authors", ACCOUNT_ID]  | create   | User creates a new claim [^2] in the webinar
["classrooms", WEBINAR_ID, ATTRIBUTE, TYPE, "authors", ACCOUNT_ID] | create   | User alters an event [^2] somehow
["classrooms", WEBINAR_ID, "rtcs"]                                 | create   | User creates an RTC
["classrooms", WEBINAR_ID, "rtcs", RTC_ID]                         | update   | User streams in an RTC
["classrooms", WEBINAR_ID]                                         | upload   | User uploads original recording
["classrooms", WEBINAR_ID]                                         | download | User downloads thumbnailed recording
["classrooms", WEBINAR_ID, "content"]                              | update   | User uploads or deletes some content (pictures, pdfs etc)

[^1]: This is both time updates for a tenant, stop for a running webinar and media-editor access

[^2]: Types, claims and attributes are documented [separately](./events.md)
