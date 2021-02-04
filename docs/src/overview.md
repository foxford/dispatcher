# Overview

Dispatcher service accepts connections at some http route and redirects to different routes based on request params.
Expected to serve different frontends based on different scopes.

## Default url

Is constructed from `default_frontend_base` by replacing its host with `{:tenant}.{:app}.{:default_frontend_base.host}`

### Routes
Path                                  | Method  | Description
------------------------------------- | ------- | ------------------
/healthz                              | GET     | Responds `Ok`
/scopes                               | GET     | List of all scopes
/frontends                            | GET     | List of all frontends.
/redirs/tenants/:tenant/apps/:app     | GET     | Redirects either to frontend found by scope and app or to default url.
/api/scopes/:scope/rollback           | POST    | Deletes the scope.
