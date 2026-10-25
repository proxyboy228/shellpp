git push и изменения remote — только по явной просьбе владельца.

## Тесты

В main — только код, на котором зелёное всё это (PowerShell):

```
cargo metadata --locked --format-version 1
cargo metadata --manifest-path server/Cargo.toml --locked --format-version 1
docker run --rm --network none -v "${PWD}:/src:ro" -v "$(git rev-parse --path-format=absolute --git-common-dir):/git:ro" ghcr.io/gitleaks/gitleaks:v8.30.1@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f git --config /src/.gitleaks.toml --gitleaks-ignore-path /src/.gitleaksignore --redact --verbose --exit-code 1 /git
cargo deny fetch db
cargo update --dry-run
cargo update --dry-run --manifest-path server/Cargo.toml
cargo deny --locked --offline check
cargo deny --locked --offline --manifest-path server/Cargo.toml check
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo clippy --locked --workspace -- -D warnings
$env:SHELLPP_TEST=1; $env:SHELLPP_TEST_SSHD='russh'; cargo test --locked --no-fail-fast --workspace
cargo test --locked --no-fail-fast -p shellpp-desktop --all-features
cargo check --locked --target aarch64-linux-android -p shellpp-core -p shellpp-ssh -p shellpp-sync-api -p shellpp-private-file -p shellpp-sessions -p shellpp-terminal
cargo fmt --manifest-path server/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path server/Cargo.toml --workspace --all-targets -- -D warnings
cargo clippy --locked --manifest-path server/Cargo.toml --workspace --all-targets --all-features -- -D warnings
cargo clippy --locked --manifest-path server/Cargo.toml --workspace -- -D warnings
$env:SHELLPP_TEST=1; cargo test --locked --no-fail-fast --manifest-path server/Cargo.toml --workspace
$env:SHELLPP_TEST=1; $env:SHELLPP_TEST_POSTGRES='docker'; cargo test --locked --no-fail-fast --manifest-path server/Cargo.toml --workspace
```

Всё это по очереди, с журналом каждого шага в `<target>\verify\`:
`powershell -ExecutionPolicy Bypass -File scripts\verify.ps1` (с
`-Deploy` — и тесты развёртывания). `--locked` роняет шаг при устаревшем
`Cargo.lock` вместо того, чтобы его переписать; `--no-fail-fast`
показывает падения всех тестовых бинарников, а не только первого.

Скан секретов (gitleaks) — та же версия 8.30.1, конфиг и `.gitleaksignore`,
что в workflow `secret-scan.yml` (там — архив релиза, здесь — образ,
закреплённый по дайджесту, без сети), по всей закоммиченной истории всех
ссылок (ветки, теги, stash); любая находка роняет шаг (что с ней делать —
D203, CONTRIBUTING). Каталог git монтируется отдельно: в worktree `.git` —
файл с путём Windows, который контейнер не видит; `verify.ps1` находит его
по файлам git, без вызова git. Нужен запущенный Docker.

`cargo deny` (лицензии, запреты, уязвимости, источники) — оба workspace,
конфиги `deny.toml` и `server/deny.toml`, как в задании «Licenses and
advisories» workflow `ci.yml`; версия cargo-deny та же — 0.20.2
(`cargo install --locked cargo-deny@0.20.2`; без него или с другой
версией шаги провалены). Сначала `fetch db` скачивает базу RustSec (она
общая для обоих конфигов), а `cargo update --dry-run` обновляет кэш индекса
crates.io для проверки отозванных (yanked) крейтов, не трогая `Cargo.lock`;
потом обе проверки идут с `--offline`. Без сети `deny-fetch` и `deny-index`
— WARNING, а не провал: проверки идут по уже скачанным базе и индексу,
и предупреждение есть в итоговой строке; если базы нет совсем, проверки
провалены. Находка —
обновить зависимость; нет обновления — исключение в конфиге с причиной
и сроком пересмотра и решение в decisions.md (D210).

Проверка «mobile-ready» (`docs/architecture.md`, 2.3) — `cargo check`
крейтов, общих с телефонным клиентом, под `aarch64-linux-android` (Samsung
S25): `verify.ps1` находит NDK (`ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, иначе
новейший в `ndk\` SDK из `ANDROID_HOME`, `ANDROID_SDK_ROOT` или
`%LOCALAPPDATA%\Android\Sdk`) и задаёт для шага `CC_aarch64_linux_android`
и `AR_aarch64_linux_android` (clang NDK с API 26 и `llvm-ar`) — для C-части
SQLite и ring. Без NDK шаг — SKIPPED с причиной в итоговой строке, а не
зелёная отметка; без цели Rust (`rustup target add aarch64-linux-android`)
— провал.

clippy без `--all-targets` видит код как release-сборка: с тестами
dev-зависимости включают `test-hooks` в `shellpp-core` для всего workspace
(объединение фич), и предупреждение в коде, нужном только хукам, не видно
(прогон 11).

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
powershell -ExecutionPolicy Bypass -File scripts\verify-linux.ps1 -AllFeatures
powershell -ExecutionPolicy Bypass -File scripts\test-process.ps1
```

Скриншоты — в `target\ui-screens\` (`<W>x<H>-<тема>-<состояние>.png`).
`verify-linux.ps1` — сборка и тесты клиента на Linux в Docker (образ
`tests/linux/Dockerfile`, тома `shellpp-linux-target` и `shellpp-linux-cargo`),
журналы — в `target\verify-linux\`.
`test-process.ps1` — тест запуска шагов (`scripts\process.ps1`): шаг ждёт
только свою программу, а не оставленные ею процессы. `Start-Process -Wait`
в PowerShell 5.1 ждёт и их, а линкер MSVC оставляет `vctip.exe`, который
может висеть четверть часа (прогон 12).

Android (`apps/android`, run 13; `docs/mobile/`): Gradle-проект
с wrapper (дистрибутив Gradle с `distributionSha256Sum`), модуль `app`
(Kotlin), Rust-связка — крейт `shellpp-android` (`apps/android/native`,
член workspace: его тесты идут и в `cargo test --workspace`). Gradle сам
собирает `.so` (`cargo build --profile android` под
`aarch64-linux-android` и `x86_64-linux-android` с clang NDK). Проверка —
в полный критерий не входит (в нём только «mobile-ready»):

```
powershell -ExecutionPolicy Bypass -File scripts\verify-android.ps1
powershell -ExecutionPolicy Bypass -File scripts\verify-android.ps1 -Emulator
```

Без `-Emulator`: тесты связки на компьютере, debug-APK, модульные тесты
Kotlin. С `-Emulator` ещё сквозной тест на эмуляторе против OpenSSH
в Docker (эмулятор видит компьютер как `10.0.2.2`): скрипт поднимает
контейнер (`tests/openssh/start.sh`, нужен bash из Git for Windows), сам
запускает AVD (`-Avd`, по умолчанию первый), если эмулятор не запущен,
и после теста гасит то, что запустил. SDK и JDK находит сам
(`ANDROID_HOME`, `JAVA_HOME`, иначе Android Studio) и задаёт их только
своему процессу. Журналы — в `target\verify-android\`. Лицензии SDK за
владельца не принимать; Gradle тянет зависимости только из Google Maven
и Maven Central.

Проверка развёртывания — после изменений в `server/`, `crates/sync-api`,
`crates/core` и `crates/private-file` (из них собираются образ сервера
и клиенты теста), в файлах
Docker (`server/Dockerfile`, `server/deploy/`) и в скрипте бэкапа. В полный
критерий не входит; нужны запущенный Docker и сеть (образы, apk,
crates.io). Тесты собирают образы, поднимают свои контейнеры с метками
`shellpp-test=deploy` и `shellpp-run=<прогон>` и удаляют их:

```
$env:SHELLPP_TEST_DEPLOY='docker'; cargo test --locked --no-fail-fast --manifest-path server/Cargo.toml --test deploy -- --test-threads=1
```

В CI они идут в своём workflow `server-deploy.yml` при изменениях в
`server/`, `crates/sync-api`, `crates/core`, `crates/private-file`,
корневом `Cargo.toml` и `rust-toolchain.toml`.

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
