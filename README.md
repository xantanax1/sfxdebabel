<div align="center">

# SFX de Babel

**Поиск звуковых эффектов по форме волны, встроенный Pure-Rust DSP FX Rack и мгновенный Drag-and-Drop в 24-bit WAV**

[![Rust](https://img.shields.io/badge/Rust-2021-141416?style=flat-square&logo=rust&logoColor=ededed)](https://www.rust-lang.org/)
[![Tauri v2](https://img.shields.io/badge/Tauri-v2-141416?style=flat-square&logo=tauri&logoColor=ededed)](https://v2.tauri.app/)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.7-141416?style=flat-square&logo=typescript&logoColor=ededed)](https://www.typescriptlang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-141416?style=flat-square)](./LICENSE)

</div>

---

**SFX de Babel** — десктопное приложение для видеомонтажёров и саунд-дизайнеров на базе **Rust**, **Tauri v2** и **Vanilla TypeScript**. Ищите сэмплы по тегам (включая запросы на русском), подбирайте звуки по нарисованной огибающей громкости, обрабатывайте их в реальном времени через встроенный DSP-рэк и перетаскивайте готовые `.wav` прямо на таймлайн Premiere Pro, DaVinci Resolve, CapCut или FL Studio.

---

## Ключевые возможности

### 1. Поиск по огибающей громкости (Envelope / RMS Matching)
- **Интерактивный Scope на HTML5 Canvas:** нарисуйте нужную динамику звука (резкий пик атаки для удара/клика или плавный горб для вуша/райзера) или выберите готовый пресет (`Whoosh`, `Impact`, `Sub Drop`, `UI Click`, `Riser`).
- **Pure-Rust DSP-анализ без системного FFmpeg:** превью декодируются в памяти (`std::io::Cursor<Bytes>`) через `symphonia` (MP3, OGG/Vorbis, WAV, FLAC, PCM) и параллельно обрабатываются в пуле `rayon` для расчёта 64-точечной нормализованной RMS-огибающей.
- **Гибридное ранжирование:** косинусное сходство + обратная RMSE с допуском лага `±2` бина (компенсирует encoder padding) + живой пересчёт процентов совпадения на лету.

### 2. Неразрушающий Audio FX Rack (Pure Rust)
- **Reverse:** покадровый разворот аудио с сохранением стереоканалов.
- **Speed / Pitch (`0.50x–2.00x`):** кубическая 4-точечная интерполяция Catmull-Rom с антиалиасинговым ФНЧ при ускорении (`Slowed` / `Speedup`).
- **Biquad EQ (`Lowpass` / `Highpass`):** биквадратные фильтры второго порядка (Audio EQ Cookbook) с независимым состоянием на канал.
- **Algorithmic Stereo Freeverb:** 8 параллельных comb-фильтров с демпфированием и 4 последовательных all-pass фильтра со стерео-сдвигом, автоапмиксом моно в стерео и хвостом реверберации.
- **Drive & Limiter:** мягкий сатуратор `tanh` с компенсацией усиления, `Fade In / Fade Out`, пиковая нормализация к `-0.3 dBFS` и лимитер `[-0.98, 0.98]`.
- **6 пресетов в 1 клик:** `Clean (Reset)`, `Slowed + Reverb`, `Speedup`, `Reverse Riser`, `Muffled / Underwater`, `Heavy Impact`.
- **Live-вейвформа:** изменение любого слайдера с debounce `80 ms` пересчитывает 64-точечный график карточки и пунктирный контур на верхнем осциллографе; двойной клик сбрасывает параметр в дефолт.

### 3. Бесконечная прокрутка и модульные источники
- **Работа «из коробки» без токенов:** в режиме `Auto` без API-ключа поиск стартует с **Mixkit**, а по окончании выдачи или на узких запросах плавно подгружает следующие страницы из публичного каталога **Freesound Web**.
- **Freesound API v2:** при добавлении токена в настройках открывается прямой поиск по всей базе Freesound с серверной фильтрацией длительности.
- **Словарь RU → EN и кэш:** встроенный переводчик популярных SFX-терминов на русском (`удар`, `вуш`, `клик`, `выстрел`, `саб дроп` и т.д.), SQLite-кэш метаданных и 64-точечных векторов (`rusqlite` с `bundled`), а также LRU-кэш декодированных буферов в RAM.

### 4. Нативный Drag-and-Drop 24-bit WAV
- Перетаскивание любой карточки за пределы окна (через `tauri-plugin-drag`) рендерит звук с текущими настройками FX в **24-bit PCM WAV** (`hound`) во временную папку ОС с описательным суффиксом (`whoosh_slowed_rev_....wav`) и передаёт файл напрямую в монтажку или DAW.

---

## Архитектура проекта

```text
sfxdebabel/
├── src/                        # Frontend (Vanilla TypeScript, HTML5 Canvas, CSS)
│   ├── index.html              # Студийный 3-панельный интерфейс (Sidebar + Stage + FX Rack)
│   ├── styles.css              # Graphite Dark Glassmorphism
│   └── main.ts                 # Canvas Envelope Scope, Infinite Scroll, FX Rack, Drag-and-Drop
├── src-tauri/                  # Rust Backend (Tauri v2)
│   ├── tauri.conf.json         # Безрамочное прозрачное окно + настройки сборки
│   ├── capabilities/           # Права окна, resize и tauri-plugin-drag
│   └── src/
│       ├── audio/
│       │   ├── decoder.rs      # Symphonia декодер из RAM-буфера
│       │   ├── envelope.rs     # Параллельный расчёт 64-pt RMS (Rayon) и матчинг
│       │   ├── fx.rs           # Pure-Rust DSP: Catmull-Rom, Biquad EQ, Freeverb, Drive, Limiter
│       │   ├── player.rs       # Выделенный поток воспроизведения и перемотки (Rodio)
│       │   └── export.rs       # Экспорт в 16-bit / 24-bit WAV (Hound)
│       ├── providers/
│       │   ├── mod.rs          # Трейт SfxProvider и RU→EN нормализатор запросов
│       │   ├── freesound.rs    # Freesound API v2
│       │   ├── freesound_web.rs# Публичный веб-источник Freesound (без ключа)
│       │   └── mixkit.rs       # Парсер Mixkit с пагинацией
│       ├── db.rs               # Локальная БД и кэш (rusqlite bundled)
│       ├── models.rs           # Общие структуры IPC
│       ├── error.rs            # Обработка ошибок (thiserror)
│       ├── commands.rs         # Tauri-команды и LRU-кэш
│       └── lib.rs              # Инициализация приложения и плагинов
├── package.json
├── tsconfig.json
└── vite.config.ts
```

---

## Технологический стек

| Слой | Технологии |
| :--- | :--- |
| **Desktop Core** | Rust (2021), Tauri v2, `tauri-plugin-drag`, `tokio` |
| **Аудио и DSP** | `symphonia`, `rodio`, `hound`, `rayon` (Pure Rust, без внешнего FFmpeg) |
| **Сеть и парсинг** | `reqwest` (`rustls-tls`), `scraper`, `serde`, `serde_json` |
| **Хранение и кэш** | `rusqlite` (`bundled` SQLite + WAL) |
| **Frontend** | Vanilla TypeScript, HTML5 Canvas, CSS, Vite |

---

## Сборка и запуск

### Требования
- **Rust** (1.82+ / edition 2021) и MSVC Build Tools на Windows
- **Node.js** 18+

### Установка и запуск в режиме разработки

```bash
git clone https://github.com/xantanax1/sfxdebabel.git
cd sfxdebabel
npm install
npm run tauri dev
```

*(В Windows PowerShell при ограниченной ExecutionPolicy используйте `npm.cmd install` и `npm.cmd run tauri dev`.)*

### Сборка релиза и тесты

```bash
# Проверка и сборка фронтенда
npm run build

# Проверка и модульные тесты Rust (DSP, WAV roundtrip, парсеры)
cd src-tauri
cargo check
cargo test

# Сборка установщика / бинарника Tauri
npm run tauri build
```

---

## Горячие клавиши

| Клавиша / Жест | Действие |
| :--- | :--- |
| `Ctrl + K` / `Cmd + K` | Фокус на строку поиска |
| `Space` | Воспроизвести / остановить выбранный звук |
| `Клик по вейвформе` | Перемотка на выбранную позицию с текущими FX |
| `Двойной клик по слайдеру` | Сброс параметра FX Rack в значение по умолчанию |
| `Перетаскивание карточки` | Экспорт 24-bit WAV и Drag-and-Drop в DAW / NLE |

---

## Лицензия

Проект распространяется под лицензией [MIT](./LICENSE).
