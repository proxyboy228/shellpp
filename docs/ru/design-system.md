# Дизайн-система

> Собственный визуальный стиль приложения. Все цвета задаются токенами.
> В коде компонентов нет «сырых» hex-значений, только токены из темы (раздел 7).

Изменения по решениям этапа 0 (`docs/decisions.md`, 24–26) помечены
[решение].

## 1. Принципы

- Основа — тёплый нейтральный графит без синего оттенка. Акцент один:
  **jade**, бирюзово-зелёный.
- Акцент используется только для главного действия на экране, фокуса,
  выделения и активного состояния. Для декора он не используется.
- Опасные действия обозначаются только цветом `danger`, никогда акцентом.
- Контраст обычного текста не ниже 4.5:1 (WCAG AA), крупного текста и иконок
  не ниже 3:1. `text-muted` разрешён только для плейсхолдеров и второстепенных
  подписей, которые повторяют информацию, показанную в другом месте; его порог
  3:1. [решение]
- Проверяемые пары и их пороги перечислены в 7.3. Контраст считается по формуле
  WCAG 2.x и сравнивается без округления. [решение]
- Тема интерфейса: Auto (как в системе), Light или Dark.

---

## 2. Токены интерфейса

### 2.1 Поверхности

| Токен | Назначение | Тёмная | Светлая |
|---|---|---|---|
| `--bg-base` | фон окна, верхняя панель с вкладками | `#121211` | `#F2F1EC` |
| `--bg-sidebar` | боковая панель | `#171716` | `#ECEBE5` |
| `--bg-surface` | основная область контента | `#1B1B19` | `#F8F7F3` |
| `--bg-raised` | карточки, блоки правой панели | `#242422` | `#FFFFFF` |
| `--bg-raised-hover` | наведение на карточку, строку или кнопку | `#2C2C29` | `#F0EFEA` |
| `--bg-pressed` | нажатая карточка, строка или кнопка [решение] | `#343431` | `#EDECE6` |
| `--bg-input` | поля ввода внутри блоков | `#1E1E1C` | `#F6F5F1` |
| `--bg-overlay` | меню, выпадающие списки, модальные окна, тосты | `#2A2A27` | `#FFFFFF` |
| `--bg-overlay-hover` | наведение на пункт меню или выпадающего списка [решение] | `#333330` | `#F0EFEA` |
| `--scrim` | затемнение под модальным окном | `rgba(0,0,0,0.55)` | `rgba(28,28,26,0.35)` |

- Семь базовых поверхностей: `--bg-base`, `--bg-sidebar`, `--bg-surface`,
  `--bg-raised`, `--bg-raised-hover`, `--bg-input` и `--bg-overlay`.
  `--bg-pressed` и `--bg-overlay-hover` — поверхности состояний. [решение]
- `--bg-pressed` в тёмной теме светлее hover, в светлой темнее. В светлой теме
  он остаётся достаточно светлым, чтобы семантические цвета текста сохраняли
  4.5:1. [решение]
- `--bg-overlay-hover` нужен потому, что hover цветом `--bg-raised-hover` не виден
  на тёмном `--bg-overlay` (1.03:1). В светлой теме он равен
  `--bg-raised-hover`. [решение]

### 2.2 Границы

| Токен | Назначение | Тёмная | Светлая |
|---|---|---|---|
| `--border-subtle` | разделители, рамки карточек, дорожка прогресса | `#2F2F2C` | `#E2E0D9` |
| `--border-strong` | рамки полей, разделители в меню, рамки чипов, дорожка выключенного переключателя | `#3E3E3A` | `#CBC8BF` |

### 2.3 Текст

| Токен | Назначение | Тёмная | Светлая |
|---|---|---|---|
| `--text-primary` | основной текст | `#EDEDE8` | `#1C1C1A` |
| `--text-secondary` | подзаголовки карточек, подписи, колонки таблиц, иконки | `#A6A59E` | `#5A5953` |
| `--text-muted` | плейсхолдеры, второстепенные подписи | `#76756F` | `#88867F` [решение] |
| `--text-disabled` | только текст неактивных пунктов меню (см. 6) [решение] | `#55554F` | `#B3B1A9` |

- Светлый `--text-muted` был `#8C8A83` (2.89:1 на `--bg-sidebar`). Новое значение
  даёт не ниже 3.05:1 на каждой базовой поверхности. [решение]

### 2.4 Акцент

| Токен | Назначение | Тёмная | Светлая |
|---|---|---|---|
| `--accent` | главная кнопка (Connect, Run, Continue), выделение карточки, включённый переключатель, полоса активной вкладки, прогресс | `#2EC4A6` | `#13856E` [решение] |
| `--accent-hover` | наведение | `#4FD3B9` | `#0F7461` [решение] |
| `--accent-pressed` | нажатие | `#23A68C` | `#0B6051` [решение] |
| `--accent-subtle` | фон активного пункта навигации, выбранной строки, чипа активного фильтра | `rgba(46,196,166,0.14)` | `rgba(19,133,110,0.10)` [решение] |
| `--accent-text` | ссылки и текстовые кнопки («Add targets», «Share this host») | `#5AD6BE` | `#0F7461` [решение] |
| `--text-on-accent` | текст на кнопке с фоном `--accent` | `#08201B` | `#FFFFFF` |
| `--focus-ring` | обводка фокуса: 2 px, отступ 2 px | `#2EC4A6` | `#13856E` [решение] |

- Светлые `--accent` и `--focus-ring` были `#13866F`: белый текст на нём даёт
  4.4994:1 и проходит только после округления. На `#13856E` — 4.56:1. [решение]
- Светлые `--accent-hover`, `--accent-pressed` и `--accent-text` сохраняют сдвиг
  относительно основы: основа сместилась на один шаг в зелёном и синем каналах,
  и они тоже (`#0F7562` → `#0F7461`, `#0B6152` → `#0B6051`). `--accent-text`
  по-прежнему равен `--accent-hover`. [решение]

### 2.5 Семантические цвета

| Токен | Назначение | Тёмная | Светлая |
|---|---|---|---|
| `--danger` | Delete, Remove, Close в меню, ошибки | `#F0665E` [решение] | `#C0392F` |
| `--danger-hover` | опасное действие под указателем: текст или фон опасной кнопки | `#F57E77` | `#A33027` |
| `--danger-pressed` | нажатая опасная кнопка [решение] | `#ED4A41` | `#852720` |
| `--danger-subtle` | фон блокирующих предупреждений (смена ключа хоста) | `rgba(240,102,94,0.14)` [решение] | `rgba(192,57,47,0.10)` |
| `--text-on-danger` | текст на кнопке с фоном `--danger` [решение] | `#08201B` | `#FFFFFF` |
| `--warning` | предупреждения, «подключается» | `#E3B04B` | `#8D620B` [решение] |
| `--warning-subtle` | фон предупреждений | `rgba(227,176,75,0.14)` | `rgba(141,98,11,0.10)` [решение] |
| `--success` | «подключено», «правило запущено» | `#7CC46F` | `#36782F` [решение] |
| `--success-subtle` | фон сообщений об успехе [решение] | `rgba(124,196,111,0.14)` | `rgba(54,120,47,0.10)` |
| `--info` | информационные подсказки | `#7AA9DB` | `#2E6AA6` |
| `--info-subtle` | фон информационных подсказок [решение] | `rgba(122,169,219,0.14)` | `rgba(46,106,166,0.10)` |

- Тёмный `--danger` был `#F0645C`. Повторная проверка по решению 24 дала 4.46:1
  на `--bg-raised-hover`, поэтому светлота поднята на минимальный шаг, при
  котором пара проходит (4.51:1). Оттенок не изменился. [решение]
- Светлые `--warning` и `--success` были `#9A6B0C` и `#3B8233` (ниже 4.5:1 как
  текст). Новые значения дают не ниже 4.52:1 на каждой базовой поверхности. [решение]
- `--danger-pressed`: в тёмной теме основа затемняется на тот же шаг, на который
  hover её осветляет; в светлой теме он на два шага hover темнее основы. [решение]
- `--text-on-danger`: белый на тёмном `--danger` даёт только 3.10:1, поэтому в
  тёмной теме используется тот же тёмный текст, что и в `--text-on-accent`. [решение]
- Фоны `*-subtle` полупрозрачные и накладываются на поверхность под ними. Текст
  на них всегда `--text-primary`, семантический цвет даётся только иконке. В
  выделении (`--accent-subtle`) второстепенные колонки могут оставаться
  `--text-secondary`. [решение]

### 2.6 Тени

| Токен | Тёмная | Светлая |
|---|---|---|
| `--shadow-overlay` (меню, поповеры, тосты) | `0 8px 24px rgba(0,0,0,0.50)` | `0 8px 24px rgba(28,28,26,0.14)` |
| `--shadow-modal` (модальные окна) | `0 16px 48px rgba(0,0,0,0.60)` | `0 16px 48px rgba(28,28,26,0.18)` |

---

## 3. Цвета объектов

### 3.1 Метки хостов и групп

- Пользователь выбирает цвет плитки хоста или группы из 8 цветов. Цвета
  одинаковые в обеих темах.
- Иконка на плитке всегда белая (`--text-on-tag`), контраст с каждым цветом не
  ниже 3:1.

| Токен | Цвет |
|---|---|
| `--tag-graphite` | `#5B5A55` (по умолчанию для хостов и групп) |
| `--tag-jade` | `#1D8571` [решение] |
| `--tag-amber` | `#9D6C1E` [решение] |
| `--tag-coral` | `#B8503F` |
| `--tag-plum` | `#85549A` |
| `--tag-moss` | `#5A7F35` |
| `--tag-ocean` | `#3A6890` |
| `--tag-rose` | `#A84B70` |
| `--text-on-tag` | `#FFFFFF` (иконки на плитках, буквы на бейджах) [решение] |

- `--tag-jade` и `--tag-amber` были `#1E8A75` и `#A87420`: белые буквы на бейджах
  (3.2) давали 4.24:1 и 4.05:1. Новые значения дают 4.52:1 и 4.57:1. [решение]
- Иконки ОС монохромные белые на плитке хоста. Фирменные цвета
  дистрибутивов не используются.
- Группа отличается от хоста иконкой папки, а не цветом.

### 3.2 Бейджи типов форвардинга

Белая буква (`--text-on-tag`) на плитке, контраст не ниже 4.5:1:

- L (Local): `--tag-jade`;
- R (Remote): `--tag-plum`;
- D (Dynamic): `--tag-amber`.

### 3.3 Индикаторы состояния

Точка 8 px (`--size-status-dot`), контраст не ниже 3:1 на базовых поверхностях:

- подключено / запущено: `--success`;
- подключается: `--warning`;
- ошибка / обрыв: `--danger`;
- не активно: `--text-muted`.

### 3.4 Чипы тегов [решение]

- Чипы тегов нейтральные: без заливки, рамка 1 px `--border-strong`, текст
  `--text-secondary` и кнопка удаления с иконкой `--text-secondary`.
- Только чип активного фильтра получает заливку `--accent-subtle` и текст
  `--text-primary`.
- Цвета плиток (3.1) в чипах тегов не используются.

---

## 4. Темы терминала по умолчанию

Формат темы описан в 7.2. Названия ролей ниже — ключи файла темы. Цвета в
формате `#RRGGBB`; `selection` — `#RRGGBBAA` и накладывается на `background`.

### 4.1 Graphite (тёмная, по умолчанию)

| Роль | Цвет | Роль | Цвет |
|---|---|---|---|
| `background` | `#121211` | `foreground` | `#DCDCD5` |
| `cursor` | `#2EC4A6` | `cursorText` | `#121211` [решение] |
| `selection` | `#2EC4A64D` (альфа 0.30) | `selectionForeground` | не задан |
| `searchMatch` | `#4A3E1C` [решение] | `searchMatchActive` | `#6A5720` [решение] |
| `black` | `#1E1E1C` | `brightBlack` | `#62625C` [решение] |
| `red` | `#E06C64` | `brightRed` | `#F08A83` |
| `green` | `#8FC46A` | `brightGreen` | `#A9D88A` |
| `yellow` | `#E3B65A` | `brightYellow` | `#F0CC7E` |
| `blue` | `#6EA2D9` | `brightBlue` | `#8DB9E8` |
| `magenta` | `#C08BD6` | `brightMagenta` | `#D3A6E5` |
| `cyan` | `#4FC9B4` | `brightCyan` | `#74DCC9` |
| `white` | `#CFCFC8` | `brightWhite` | `#F5F5F0` |

- `brightBlack` был `#5A5A55` (2.70:1); новое значение даёт 3.05:1. [решение]

### 4.2 Paper (светлая)

| Роль | Цвет | Роль | Цвет |
|---|---|---|---|
| `background` | `#FAF9F6` | `foreground` | `#2A2A27` |
| `cursor` | `#13856E` [решение] | `cursorText` | `#FFFFFF` [решение] |
| `selection` | `#13856E33` (альфа 0.20) [решение] | `selectionForeground` | не задан |
| `searchMatch` | `#F2E2B0` [решение] | `searchMatchActive` | `#E5C46E` [решение] |
| `black` | `#2A2A27` | `brightBlack` | `#5C5B55` |
| `red` | `#B3403A` | `brightRed` | `#C6483F` [решение] |
| `green` | `#3F7D2E` | `brightGreen` | `#458034` [решение] |
| `yellow` | `#8E6A12` | `brightYellow` | `#916D1A` [решение] |
| `blue` | `#2F62A0` | `brightBlue` | `#3E74B6` [решение] |
| `magenta` | `#8A4AA0` | `brightMagenta` | `#9B57B3` [решение] |
| `cyan` | `#157A6A` | `brightCyan` | `#1B8070` [решение] |
| `white` | `#8A8982` | `brightWhite` | `#B9B7AF` |

- Яркие цвета были ниже 4.5:1 (bright red 3.87:1, bright green 3.41:1); новые
  значения дают не ниже 4.52:1. [решение]
- Курсор и выделение следуют светлому `--accent` (`#13856E`): со старым
  `#13866F` белый текст под курсором давал 4.4994:1. [решение]

### 4.3 Подсветка ключевых слов (цвета по умолчанию) [решение]

- Колонка выбирается по фону темы терминала, а не по теме интерфейса. Фон
  считается светлым, если чёрный текст на нём контрастнее белого (относительная
  яркость выше примерно 0.1791); иначе он тёмный.
- Цвета хранятся в hex. Значения по умолчанию дают не ниже 4.5:1 на фонах
  Graphite и Paper; цвета пользователя и импортированные темы не проверяются.
- Значения по умолчанию для Error, Warning, OK и Info равны `--danger`,
  `--warning`, `--success` и `--info` темы интерфейса той же яркости.

| Категория | Тёмный фон | Светлый фон |
|---|---|---|
| Error | `#F0665E` | `#C0392F` |
| Warning | `#E3B04B` | `#8D620B` |
| OK | `#7CC46F` | `#36782F` |
| Info | `#7AA9DB` | `#2E6AA6` |
| Debug | `#A38FD9` | `#6B55B0` |
| IP и MAC | `#D883A6` | `#A84B70` |

---

## 5. Типографика

- Интерфейс (`--font-ui`): системный шрифт (Segoe UI Variable на Windows, SF Pro
  на macOS, системный sans в Linux). Запасной вариант — встроенный Inter (OFL).
- Размеры интерфейса:
  - 12 px (`--font-size-caption`): подзаголовки карточек, подписи;
  - 13 px (`--font-size-dense`): плотные списки и таблицы;
  - 14 px (`--font-size-body`): основной текст;
  - 16 px (`--font-size-heading`): заголовки разделов («Groups», «Hosts», «Keys»);
  - 20 px (`--font-size-title`): заголовки модальных окон.
- Начертания: 400 (`--font-weight-regular`), 500 (`--font-weight-medium`),
  600 (`--font-weight-semibold`).
- Терминал, поля скриптов и ключей (`--font-mono`): встроенный JetBrains Mono
  (OFL), 14 px (`--font-size-mono`) по умолчанию, межстрочный интервал 1.2
  (`--line-height-mono`).
- JetBrains Mono встроен с этапа 2a (D30). Inter пока не встроен; набор шрифтов
  интерфейса откатывается к установленным в системе.
- Цифры в таблицах (размеры, даты, время): `font-variant-numeric: tabular-nums`.

---

## 6. Форма и отступы

- Сетка 4 px. Отступы: 4 px (`--space-1`), 8 px (`--space-2`), 12 px (`--space-3`),
  16 px (`--space-4`), 24 px (`--space-5`), 32 px (`--space-6`).
- Радиусы:
  - кнопки, поля, чипы: 6 px (`--radius-control`);
  - плитки иконок: 8 px (`--radius-tile`);
  - карточки и блоки панели: 10 px (`--radius-card`);
  - меню и модальные окна: 12 px (`--radius-overlay`).
- Высоты:
  - верхняя панель: 44 px (`--height-top-bar`);
  - кнопки и поля: 36 px (`--height-control`);
  - строки таблиц: 44 px (`--height-row`);
  - карточки хостов: 64 px (`--height-host-card`).
- Ширины:
  - боковая панель: 220 px (`--width-sidebar`); в узком окне — полоска иконок 56 px
    (UI 1.2) [решение];
  - правая панель: 380 px (`--width-details`), ширину можно менять
    перетаскиванием от 320 px (`--width-details-min`) до 560 px (`--width-details-max`), но
    не настолько, чтобы основная область стала уже 360 px; выбранная ширина возвращается,
    когда окно становится шире [решение].
- Окно: минимальный размер 960 × 600 px (UI 1, D29). При этой ширине, когда открыта
  правая панель шириной 380 px, боковая панель свёрнута в полоску иконок (UI 1.2), и на
  основную область остаётся 524 px. Любой экран помещается и в самую узкую основную
  область, которую допускает правая панель, — 360 px — без обрезки и наложений.
- Состояния:
  - hover: `--bg-raised-hover` для карточек, строк и кнопок; `--bg-overlay-hover`
    для пунктов меню и выпадающих списков [решение];
  - pressed: `--bg-pressed` [решение];
  - опасное действие под указателем или в нажатом состоянии использует текст
    `--danger-hover`; обычный текст `--danger` на поверхности состояний не
    ставится [решение];
  - `--text-muted` на поверхности состояний не ставится. Подписи и подсказки
    сочетаний клавиш в меню используют `--text-secondary` [решение];
  - disabled [решение]: весь компонент получает прозрачность 0.45
    (`--opacity-disabled`) и не реагирует на наведение. `--text-disabled`
    используется только для текста неактивных пунктов меню, которые остаются
    непрозрачными. Вместе они не применяются;
  - фокус с клавиатуры: `--focus-ring`, 2 px (`--focus-ring-width`) с отступом
    2 px (`--focus-ring-offset`).
- Индикаторы и производные состояния [решение]:
  - активная вкладка сессии: полоса 2 px (`--size-tab-indicator`) цвета
    `--accent`; цвета темы терминала хоста для неё не используются;
  - переключатель: включён — дорожка `--accent`; выключен — дорожка
    `--border-strong`;
  - прогресс: заливка `--accent` на дорожке `--border-subtle`;
  - тост: `--bg-overlay` с `--shadow-overlay`.
- Анимации: от 120 мс (`--duration-fast`: наведение, нажатие, переключатели) до
  180 мс (`--duration-normal`: меню, панели, модальные окна), ease-out
  (`--easing-standard`). При `prefers-reduced-motion: reduce` длительности равны
  0 мс, поэтому компоненты не должны ждать `transitionend` или `animationend`.

---

## 7. Реализация

### 7.1 Тема

- Тема интерфейса идёт через систему тем gpui-component, которую
  реэкспортирует gpui-kit: глобальный `Theme` и его поля `ThemeColor`.
  Компоненты кита (поля ввода, меню, вкладки, боковая панель, заголовок окна,
  полосы прокрутки, диалоги) рисуются по ней, а наши view читают те же значения
  через `cx.theme()`.
- Файл темы — `apps/desktop/themes/graphite.json` в схеме тем gpui-component:
  один набор тем Graphite с двумя темами, `"mode": "dark"` и `"mode": "light"`.
  Его значения совпадают с этим документом. Соответствие наших токенов полям
  `ThemeColor` — в 7.4.
- Auto следует системному оформлению; Light и Dark задают режим явно. Смена
  режима применяется сразу ко всем окнам.
- Цвета, для которых нет поля в `ThemeColor`, — константы в
  `apps/desktop/src/theme/palette.rs`:
  - цвета плиток `--tag-*` и `--text-on-tag` (3.1) и бейджи типов форвардинга
    (3.2);
  - цвета точек состояния (3.3);
  - цвета подсветки ключевых слов по умолчанию (4.3);
  - зависящие от темы токены с пометкой «palette.rs» в 7.4, у каждого тёмное
    и светлое значение.
- Запись палитры, которая повторяет другой токен, ссылается на него, а не
  дублирует значение: бейдж использует цвет `--tag-*`, а точка состояния —
  `--success`, `--warning`, `--danger` или `--text-muted`.
- Токены, которые не являются цветами (отступы, размеры, радиусы, шрифты,
  анимации), — константы Rust в `apps/desktop/src/theme/`. `graphite.json`
  также задаёт по ним базовые значения кита: `font.size` — это
  `--font-size-body`, `radius` — `--radius-control`, `radius.lg` —
  `--radius-overlay`.
- «Сырые» значения цветов разрешены только в `apps/desktop/themes/graphite.json`
  и в `apps/desktop/src/theme/`. View берут цвета из `cx.theme()` и
  `palette.rs`, а не из литералов.
- Rust-тест в `apps/desktop/src/theme` сверяет значения обоих режимов
  `graphite.json` и константы `palette.rs` с таблицами разделов 2–4 и 7.4
  и проверяет, что в `docs/ru/design-system.md` те же значения.

### 7.2 Формат темы терминала [решение]

- Встроенные темы хранятся как JSON в `apps/desktop/src/theme/terminal/`, по
  файлу на тему. Импортированные темы хранятся записями и синхронизируются;
  файлы `.itermcolors` и схемы Windows Terminal при импорте переводятся в этот
  формат.
- Тема — JSON-объект в UTF-8 со следующими полями:

| Поле | Значение | Обязательное | Смысл |
|---|---|---|---|
| `name` | строка, 1–64 символа | да | название в списках тем |
| `version` | целое `1` | да | версия формата |
| `background` | `#RRGGBB` | да | фон терминала |
| `foreground` | `#RRGGBB` | да | текст по умолчанию |
| `cursor` | `#RRGGBB` | да | курсор |
| `cursorText` | `#RRGGBB` | да | текст под блочным курсором |
| `selection` | `#RRGGBB` или `#RRGGBBAA` | да | фон выделения, накладывается на `background` |
| `selectionForeground` | `#RRGGBB` | нет | текст в выделении; если поля нет, текст сохраняет свои цвета |
| `searchMatch` | `#RRGGBB` | да | фон совпадений поиска |
| `searchMatchActive` | `#RRGGBB` | да | фон текущего совпадения поиска |
| `ansi` | объект | да | 16 цветов ANSI |

- В `ansi` 16 ключей в порядке индексов цветов 0–15: `black`, `red`, `green`,
  `yellow`, `blue`, `magenta`, `cyan`, `white`, `brightBlack`, `brightRed`,
  `brightGreen`, `brightYellow`, `brightBlue`, `brightMagenta`, `brightCyan`,
  `brightWhite`. Каждое значение — `#RRGGBB`.
- Hex-цифры могут быть в любом регистре. Других полей версия 1 не допускает:
  файл с неизвестным полем или другой `version` отклоняется с ошибкой.
- Колонка подсветки ключевых слов выбирается по `background` (4.3).
- Пример (Graphite):

```json
{
  "name": "Graphite",
  "version": 1,
  "background": "#121211",
  "foreground": "#DCDCD5",
  "cursor": "#2EC4A6",
  "cursorText": "#121211",
  "selection": "#2EC4A64D",
  "searchMatch": "#4A3E1C",
  "searchMatchActive": "#6A5720",
  "ansi": {
    "black": "#1E1E1C",
    "red": "#E06C64",
    "green": "#8FC46A",
    "yellow": "#E3B65A",
    "blue": "#6EA2D9",
    "magenta": "#C08BD6",
    "cyan": "#4FC9B4",
    "white": "#CFCFC8",
    "brightBlack": "#62625C",
    "brightRed": "#F08A83",
    "brightGreen": "#A9D88A",
    "brightYellow": "#F0CC7E",
    "brightBlue": "#8DB9E8",
    "brightMagenta": "#D3A6E5",
    "brightCyan": "#74DCC9",
    "brightWhite": "#F5F5F0"
  }
}
```

### 7.3 Пары контраста [решение]

- Автотест — Rust-тест в `apps/desktop/src/theme`, запускается через
  `cargo test`. Он читает оба режима `apps/desktop/themes/graphite.json`,
  константы `palette.rs` и встроенные темы терминала, накладывает полупрозрачные
  цвета на поверхность под ними, считает контраст по формуле WCAG 2.x
  и сравнивает его с порогом без округления.
- Список пар лежит в том же тестовом модуле и повторяет таблицы ниже; менять их
  нужно вместе.
- «Базовые поверхности» — семь поверхностей из 2.1; «все поверхности» —
  они же плюс `--bg-pressed` и `--bg-overlay-hover`.

Пары интерфейса, проверяются в обеих темах:

| Группа | Передний план | Фон | Порог |
|---|---|---|---|
| Текст | `--text-primary`, `--text-secondary`, `--accent-text`, `--danger`, `--danger-hover`, `--warning`, `--success`, `--info` | базовые поверхности | 4.5 |
| Текст на поверхностях состояний | `--text-primary`, `--text-secondary`, `--accent-text`, `--danger-hover`, `--warning`, `--success`, `--info` | `--bg-pressed`, `--bg-overlay-hover` | 4.5 |
| Главная кнопка | `--text-on-accent` | `--accent`, `--accent-hover`, `--accent-pressed` | 4.5 |
| Опасная кнопка | `--text-on-danger` | `--danger`, `--danger-hover`, `--danger-pressed` | 4.5 |
| Текст на тонированных фонах | `--text-primary` | каждый `*-subtle` поверх каждой базовой поверхности | 4.5 |
| Второстепенный текст в выделении | `--text-secondary` | `--accent-subtle` поверх `--bg-sidebar`, `--bg-surface`, `--bg-raised`, `--bg-overlay` | 4.5 |
| Буквы на бейджах | `--text-on-tag` | `--tag-jade`, `--tag-plum`, `--tag-amber` | 4.5 |
| Приглушённый текст | `--text-muted` | базовые поверхности | 3 |
| Точки состояния | `--success`, `--warning`, `--danger`, `--text-muted` | базовые поверхности | 3 |
| Фокус и индикаторы акцента | `--focus-ring`, `--accent` | все поверхности | 3 |
| Иконки на тонированных фонах | `--accent`, `--danger`, `--warning`, `--success`, `--info` | свой `*-subtle` поверх каждой базовой поверхности | 3 |
| Иконки на плитках | `--text-on-tag` | все восемь `--tag-*` | 3 |
| Прогресс | `--accent` | `--border-subtle` | 3 |

Пары терминала, проверяются для каждой встроенной темы:

| Группа | Передний план | Фон | Порог |
|---|---|---|---|
| Текст | `foreground` | `background` | 4.5 |
| Цвета ANSI 1–6 и 9–14 | `red` … `cyan`, `brightRed` … `brightCyan` | `background` | 4.5 |
| Bright black | `brightBlack` | `background` | 3 |
| Выделенный текст | `selectionForeground` или `foreground`, если он не задан | `selection` поверх `background` | 4.5 |
| Текст под курсором | `cursorText` | `cursor` | 4.5 |
| Курсор | `cursor` | `background` | 3 |
| Совпадения поиска | `foreground` | `searchMatch`, `searchMatchActive` | 4.5 |
| Подсветка ключевых слов | значения колонки, выбранной по `background` (4.3) | `background` | 4.5 |

- Иконки интерфейса используют `--text-primary`, `--text-secondary`, `--accent`
  или семантический цвет, поэтому они покрыты парами выше.
- Не проверяются:
  - `--text-disabled` и всё, что показано с прозрачностью 0.45 (неактивные
    компоненты);
  - `--text-muted` и обычный `--danger` на поверхностях состояний, где они не
    используются (6);
  - ANSI `black`, `white` и `brightWhite` тем терминала, а также цвета ANSI на
    выделении и на совпадениях поиска;
  - импортированные темы и цвета, выбранные пользователем.

### 7.4 Токены и поля `ThemeColor`

- Имена полей — из `ThemeColor` в gpui-component 0.7.3; последняя колонка —
  ключ в `graphite.json`. Оба режима используют одно и то же соответствие.
- `accent` в gpui-component — это **не** наш акцент. Это фон пункта меню или
  списка под указателем или при выделении с клавиатуры (пункты меню, списки
  команд и автодополнения, наведение на ghost-кнопку). Наш `--accent`
  соответствует `primary` и полям `button_primary`; в `accent` идёт
  `--bg-overlay-hover`.
- В `muted_foreground` идёт `--text-secondary`, а не `--text-muted`: кит
  использует это поле для подсказок сочетаний клавиш в меню, второстепенных
  подписей, текста вкладок и иконок, а для них раздел 6 требует
  `--text-secondary`.
- «palette.rs» значит, что у токена нет поля в `ThemeColor` (7.1).
- Поле, которого нет в таблице, нашими экранами не используется.
  `graphite.json` всё равно задаёт его ближайшим по роли токеном, чтобы нигде
  не появлялся цвет кита по умолчанию.
- Если компонент кита рисует поверхность или состояние не так, как в разделах 2
  и 6 (например, диалог кита использует `background`, а фон поля ввода кит
  выводит из `input`), этот цвет задаёт по токенам наш view или
  компонент-обёртка.

| Токен | Поля `ThemeColor` | Ключи в `graphite.json` |
|---|---|---|
| `--bg-base` | `background`, `title_bar`, `tab_bar` | `background`, `title_bar.background`, `tab_bar.background` |
| `--bg-sidebar` | `sidebar` | `sidebar.background` |
| `--bg-surface` | `list`, `table` | `list.background`, `table.background` |
| `--bg-raised` | `secondary`, `muted`, `group_box` | `secondary.background`, `muted.background`, `group_box.background` |
| `--bg-raised-hover` | `secondary_hover`, `button_hover`, `list_hover`, `table_hover` | `secondary.hover.background`, `button.hover.background`, `list.hover.background`, `table.hover.background` |
| `--bg-pressed` | `secondary_active`, `button_active` | `secondary.active.background`, `button.active.background` |
| `--bg-input` | palette.rs | — |
| `--bg-overlay` | `popover` | `popover.background` |
| `--bg-overlay-hover` | `accent` | `accent.background` |
| `--scrim` | `overlay` | `overlay` |
| `--border-subtle` | `border`, `sidebar_border`, `title_bar_border`, `window_border`, `table_row_border` | `border`, `sidebar.border`, `title_bar.border`, `window.border`, `table.row.border` |
| `--border-strong` | `input`, `switch` | `input.border`, `switch.background` |
| `--text-primary` | `foreground`, `accent_foreground`, `popover_foreground`, `secondary_foreground`, `sidebar_foreground`, `sidebar_accent_foreground`, `tab_active_foreground` | `foreground`, `accent.foreground`, `popover.foreground`, `secondary.foreground`, `sidebar.foreground`, `sidebar.accent.foreground`, `tab.active.foreground` |
| `--text-secondary` | `muted_foreground`, `tab_foreground` | `muted.foreground`, `tab.foreground` |
| `--text-muted` | palette.rs | — |
| `--text-disabled` | palette.rs | — |
| `--accent` | `primary`, `button_primary`, `sidebar_primary`, `progress_bar`, `caret` | `primary.background`, `button.primary.background`, `sidebar.primary.background`, `progress.bar.background`, `caret` |
| `--accent-hover` | `primary_hover`, `button_primary_hover` | `primary.hover.background`, `button.primary.hover.background` |
| `--accent-pressed` | `primary_active`, `button_primary_active` | `primary.active.background`, `button.primary.active.background` |
| `--accent-subtle` | `list_active`, `table_active`, `sidebar_accent`, `tab_active` | `list.active.background`, `table.active.background`, `sidebar.accent.background`, `tab.active.background` |
| `--accent-text` | `link`, `link_hover`, `link_active` | `link`, `link.hover`, `link.active` |
| `--text-on-accent` | `primary_foreground`, `button_primary_foreground`, `sidebar_primary_foreground` | `primary.foreground`, `button.primary.foreground`, `sidebar.primary.foreground` |
| `--focus-ring` | `ring` | `ring` |
| `--danger` | `danger`, `button_danger` | `danger.background`, `button.danger.background` |
| `--danger-hover` | `danger_hover`, `button_danger_hover` | `danger.hover.background`, `button.danger.hover.background` |
| `--danger-pressed` | `danger_active`, `button_danger_active` | `danger.active.background`, `button.danger.active.background` |
| `--danger-subtle` | palette.rs | — |
| `--text-on-danger` | `danger_foreground`, `button_danger_foreground` | `danger.foreground`, `button.danger.foreground` |
| `--warning` | `warning` | `warning.background` |
| `--warning-subtle` | palette.rs | — |
| `--success` | `success` | `success.background` |
| `--success-subtle` | palette.rs | — |
| `--info` | `info` | `info.background` |
| `--info-subtle` | palette.rs | — |
| `--shadow-overlay`, `--shadow-modal` | palette.rs (в теме кита есть только переключатель `shadow`) | — |
| `--tag-*`, `--text-on-tag` | palette.rs | — |
