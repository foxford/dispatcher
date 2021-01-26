# Overview

Dispatcher service accepts connections at some http route and redirects to different routes based on request params.
Expected to serve different frontends based on different scopes.

### Routes
Path       | Method  | Description
---------- | ------- | ------------------
/healthz   | GET     | Responds `Ok`
/scopes    | GET     | List of all scopes
/frontends | GET     | List of all frontends.
/redir     | GET     | Redirects either to frontend found by scope or to `default_frontend` in config.
