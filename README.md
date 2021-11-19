# Dispatcher

[![dependency status](https://deps.rs/repo/github/foxford/dispatcher/status.svg)](https://deps.rs/repo/github/foxford/dispatcher)

Сервис, который:
* обслуживает единый http роут, редиректя на разные версии фронтов
* по скоупу решает на какой из фронтов нужен редирект
* может оповестить всех (кроме паблишера?) людей в топике скоупа, что им нужно рефрешнуть страницу
