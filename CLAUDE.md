git push и изменения remote — только по явной просьбе владельца.

## Тесты

В main — только код, на котором зелёное всё это (PowerShell):

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
$env:SHELLPP_TEST=1; $env:SHELLPP_TEST_SSHD='russh'; cargo test --workspace
cargo test -p shellpp-desktop --all-features
cargo fmt --manifest-path server/Cargo.toml --all -- --check
cargo clippy --manifest-path server/Cargo.toml --workspace --all-targets -- -D warnings
cargo clippy --manifest-path server/Cargo.toml --workspace --all-targets --all-features -- -D warnings
$env:SHELLPP_TEST=1; cargo test --manifest-path server/Cargo.toml --workspace
$env:SHELLPP_TEST=1; $env:SHELLPP_TEST_POSTGRES='docker'; cargo test --manifest-path server/Cargo.toml --workspace
```

Сервер (`server/`) — отдельный workspace, корневые команды его не видят;
последние пять строк — его критерий: тесты на SQLite и они же на
PostgreSQL в Docker. Пока в `server/` нет `Cargo.toml` (до этапа 7),
эти строки пропускаются.

Отдельно:

```
$env:SHELLPP_TEST_PUTTYGEN='docker'; cargo test -p shellpp-core --test key_formats_putty
powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -States export-data,import-data
powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -Stage3
powershell -ExecutionPolicy Bypass -File scripts\measure-terminal.ps1 -Server russh -Only output -SeqLines 3000000
powershell -ExecutionPolicy Bypass -File scripts\measure-startup.ps1
```

Скриншоты — в `target\ui-screens\` (`<W>x<H>-<тема>-<состояние>.png`).

Наборы против OpenSSH в Docker:

```
bash tests/openssh/start.sh <папка>
```

затем с `SHELLPP_TEST=1`, `SHELLPP_TEST_SSHD=openssh` и
`SHELLPP_OPENSSH_SERVER_JSON=<папка>/server.json`:

```
cargo test -p shellpp-ssh
cargo test -p shellpp-desktop --test sessions -- --test-threads=1
```

После прогона удалить контейнер: `docker rm -f $(cat <папка>/container)`.

## Подводные камни

- Тесты сессий на общем контейнере OpenSSH — только с `--test-threads=1`:
  одни тесты роняют соединения и меняют ключ хоста, соседние от этого
  падают. В CI у каждого теста свой контейнер.
- Контейнер живёт `SHELLPP_OPENSSH_LIFETIME` секунд (по умолчанию 3600,
  задаётся при `start.sh`) и может умереть посреди длинного прогона —
  перед длинным прогоном поднимать заново или задавать больше.
- Переменные окружения:
  - `SHELLPP_TEST=1` — тестовый запуск: каталог данных берётся только из
    `SHELLPP_DATA_DIR`, настоящие `~/.ssh` и агент не используются;
  - `SHELLPP_TEST_SSHD` — тестовый SSH-сервер: `russh` (встроенный) или
    `openssh` (Docker, вместе с `SHELLPP_OPENSSH_SERVER_JSON`). Пусто или
    `auto` — тесты desktop сами берут OpenSSH в Docker, если Docker
    запущен, иначе russh; поэтому `cargo test --workspace` — с явным
    `russh`, а не на общем контейнере с параллельными тестами;
  - `SHELLPP_TEST_PUTTYGEN=docker` — тесты форматов против настоящего
    puttygen;
  - `SHELLPP_TEST_POSTGRES=docker` — тесты сервера на PostgreSQL: сами
    поднимают контейнер `postgres:17-alpine` и удаляют его; без
    переменной — на SQLite;
  - `SHELLPP_SFTP_BENCH=1` — замер SFTP (release-сборка).
- Диск C почти заполнен: перед сборкой проверять свободное место; при
  нехватке — удалить `target\debug\incremental`, потом `cargo clean`.
- Полный запуск `scripts\ui-screens.ps1` идёт дольше 10 минут. Прежний
  сторож в 600 с считал от начала всего запуска и срабатывал там, где
  запуск оказывался к этому времени (в прогоне 3 — на `new-tab-search`):
  это было не зависание. Теперь у каждого состояния 120 с на тему и один
  повтор, сторож срабатывает после 300 с без прогресса, а скрипт ждёт
  запуск до часа (D72). Не возвращать общий лимит на весь запуск.
