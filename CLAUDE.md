git push и изменения remote — только по явной просьбе владельца.

## Тесты

В main — только код, на котором зелёное всё это (PowerShell):

```
cargo metadata --locked --format-version 1
cargo metadata --manifest-path server/Cargo.toml --locked --format-version 1
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
$env:SHELLPP_TEST=1; $env:SHELLPP_TEST_SSHD='russh'; cargo test --locked --no-fail-fast --workspace
cargo test --locked --no-fail-fast -p shellpp-desktop --all-features
cargo fmt --manifest-path server/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path server/Cargo.toml --workspace --all-targets -- -D warnings
cargo clippy --locked --manifest-path server/Cargo.toml --workspace --all-targets --all-features -- -D warnings
$env:SHELLPP_TEST=1; cargo test --locked --no-fail-fast --manifest-path server/Cargo.toml --workspace
$env:SHELLPP_TEST=1; $env:SHELLPP_TEST_POSTGRES='docker'; cargo test --locked --no-fail-fast --manifest-path server/Cargo.toml --workspace
```

Всё это по очереди, с журналом каждого шага в `<target>\verify\`:
`powershell -ExecutionPolicy Bypass -File scripts\verify.ps1` (с
`-Deploy` — и тесты развёртывания). `--locked` роняет шаг при устаревшем
`Cargo.lock` вместо того, чтобы его переписать; `--no-fail-fast`
показывает падения всех тестовых бинарников, а не только первого.

Сервер (`server/`) — отдельный workspace, корневые команды его не видят;
строки с `server/Cargo.toml` — его критерий: тесты на SQLite и они же на
PostgreSQL в Docker. В CI эти тесты идут в workflow `server.yml` при тех же
изменениях, что и тесты развёртывания (D202).

Отдельно:

```
$env:SHELLPP_TEST_PUTTYGEN='docker'; cargo test -p shellpp-core --test key_formats_putty
powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -States export-data,import-data
powershell -ExecutionPolicy Bypass -File scripts\ui-screens.ps1 -Sizes 1280x800 -Stage3
powershell -ExecutionPolicy Bypass -File scripts\measure-terminal.ps1 -Server russh -Only output -SeqLines 3000000
powershell -ExecutionPolicy Bypass -File scripts\measure-startup.ps1
```

Скриншоты — в `target\ui-screens\` (`<W>x<H>-<тема>-<состояние>.png`).

Проверка развёртывания — после изменений в `server/`, `crates/sync-api` и
`crates/core` (из них собираются образ сервера и клиенты теста), в файлах
Docker (`server/Dockerfile`, `server/deploy/`) и в скрипте бэкапа. В полный
критерий не входит; нужны запущенный Docker и сеть (образы, apk,
crates.io). Тесты собирают образы, поднимают свои контейнеры с метками
`shellpp-test=deploy` и `shellpp-run=<прогон>` и удаляют их:

```
$env:SHELLPP_TEST_DEPLOY='docker'; cargo test --locked --no-fail-fast --manifest-path server/Cargo.toml --test deploy -- --test-threads=1
```

В CI они идут в своём workflow `server-deploy.yml` при изменениях в
`server/`, `crates/sync-api`, `crates/core`, корневом `Cargo.toml` и
`rust-toolchain.toml`.

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

- russh собирается из `third_party/russh` через `[patch.crates-io]`
  в корневом `Cargo.toml`: 0.64.1 с двумя заплатками
  (`third_party/russh/PATCHES.md`, в коде помечены «Shell++ patch»):
  1. утечка каналов (D100) — `src/client/encrypted.rs`, тесты
     `crates/ssh/tests/channel_close.rs`, текст для upstream
     `docs/upstream/russh-channel-leak.md`;
  2. пустой `CHANNEL_DATA` читается как конец потока (D191) —
     `src/channels/io/rx.rs`, тесты `crates/ssh/tests/empty_channel_data.rs`,
     текст для upstream `docs/upstream/russh-empty-data.md`.

  Тесты обеих заплаток остаются всегда. Когда релиз upstream исправит
  обе — перейти на него и убрать каталог `third_party/russh`, запись
  `[patch.crates-io]`, исключение в `.gitleaks.toml` и абзац в
  THIRD_PARTY_NOTICES. Когда только одну — каталог остаётся: заменить его
  содержимое исходниками этого релиза, заново наложить оставшуюся
  заплатку и правку `Cargo.toml` копии (п. 3 `PATCHES.md`), убрать
  исправленную из `PATCHES.md`, поправить версию и число заплаток в этом
  абзаце, `PATCHES.md`, корневом `Cargo.toml`, `.gitleaks.toml`
  и THIRD_PARTY_NOTICES.

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
  - `SHELLPP_TEST_POSTGRES=docker` — тесты сервера на PostgreSQL: каждый
    тестовый бинарник поднимает свой контейнер `postgres:17-alpine`
    (метки `shellpp-test=postgres` и `shellpp-run=<прогон>`); контейнер
    останавливается и удаляется вместе с томом, как только процесс
    бинарника завершился, как бы он ни завершился (контейнер ждёт конца
    своего stdin). Без переменной — на SQLite;
  - `SHELLPP_TEST_RUN` — имя прогона или агента (`run10`,
    `run11-a`): все тестовые контейнеры (PostgreSQL, развёртывание,
    OpenSSH) получают метку `shellpp-run=<имя>`, по умолчанию `local`;
    `tests/openssh/stop.sh --all` удаляет только контейнеры своего имени.
    Найти свои: `docker ps -a --filter label=shellpp-run=<имя>`;
  - `SHELLPP_SFTP_BENCH=1` — замер SFTP (release-сборка).
- Перед сборкой проверять свободное место на диске C (пороги — ROADMAP,
  раздел 7.5): при нехватке — удалить `target\debug\incremental`, потом
  `cargo clean`.
- Полный запуск `scripts\ui-screens.ps1` идёт дольше 10 минут. Прежний
  сторож в 600 с считал от начала всего запуска и срабатывал там, где
  запуск оказывался к этому времени (в прогоне 3 — на `new-tab-search`):
  это было не зависание. Теперь у каждого состояния 120 с на тему и один
  повтор, сторож срабатывает после 300 с без прогресса, а скрипт ждёт
  запуск до часа (D72). Не возвращать общий лимит на весь запуск.
