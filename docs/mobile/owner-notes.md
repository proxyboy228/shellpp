# The owner's description of the mobile app (D211)

The owner's own words about the Android app, recorded in run 13 (2026-10-25):
in Russian as said, with an English translation beside it. The mobile
specifications (`docs/mobile/`) take the app's behaviour only from these
words and from the accepted desktop specifications; whatever they add is
marked [decision] there.

## Русский (как сказано)

**Сценарий.** Телефон — Samsung S25, Android. Изредка, не из дома, зайти
по SSH на свои серверы и сделать что нужно. Синхронизация с его сервером
желательна полная, с первого дня. Сервер синхронизации доступен по
публичному домену (Let's Encrypt).

**Что делает на серверах.** Смотрит логи, перезапускает сервисы, правит
конфиги в nano, смотрит `htop`, скачивает файлы.

**Набор.** Нужны все доп. клавиши (Esc, Tab, Ctrl, Alt, стрелки,
символы вроде `|`, `/`, `~`, `-`). Пользуется сниппетами, историей
команд и автодополнением.

**Что мешало в прежнем клиенте** (главные требования к терминалу):
- нельзя нормально поставить курсор на нужный символ;
- трудно выделить нужный текст — например, кусок конфига, чтобы его
  удалить.

**Вход.** Разблокировка вводом пароля шифрования (мастер-пароля).

**Первая версия, основа:**
- подключение по SSH;
- группы хостов;
- хранилище ключей (Keychain);
- SFTP;
- known hosts.

**Желательно всё остальное:** сниппеты, история, автодополнение
и прочее — следующими версиями.

**Ещё:**
- iPhone не делаем;
- Mosh — позже;
- отпечаток пальца — опция, по умолчанию выключена [решение]: владелец
  назвал вход паролем.

## English (translation)

**The case.** The phone is a Samsung S25 on Android. Now and then, away from
home, SSH to one's own servers and do what is needed. Sync with the owner's
server is wanted in full, from the first day. The sync server is reachable at
a public domain (Let's Encrypt).

**What the owner does on the servers.** Reads logs, restarts services, edits
configuration files in nano, watches `htop`, downloads files.

**Typing.** Every extra key is needed (Esc, Tab, Ctrl, Alt, the arrows,
symbols such as `|`, `/`, `~`, `-`). The owner uses snippets, the command
history and autocomplete.

**What got in the way in the previous client** (the main requirements of the
terminal):
- the cursor cannot be put on the wanted character properly;
- the wanted text is hard to select, for example a piece of a configuration
  file in order to delete it.

**Entry.** Unlocking by typing the encryption password (the master password).

**The first version, the core of it:**
- SSH connections;
- host groups;
- the key store (Keychain);
- SFTP;
- known hosts.

**Everything else is wanted:** snippets, history, autocomplete and the rest,
in later versions.

**Also:**
- no iPhone app;
- Mosh comes later;
- fingerprint unlock is an option, off by default [decision]: the owner named
  the password as the way in.
