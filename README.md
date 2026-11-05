<div align="center">

<img src="https://github.com/ZigRain-hub/mimic-discord-easy/blob/main/assets/logo.png?raw=true" width="120" alt="QDCord Logo" />

# QDCord

**Лёгкий нативный эмулятор игровой активности для Discord.**
Прокачивает значки «Универсал» и «Время в играх». Без WebView, без Node.js — один exe.

[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=for-the-badge&logo=windows&logoColor=white)](#)
[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](#)
[![Win32](https://img.shields.io/badge/Win32_API-000000?style=for-the-badge&logo=windows95&logoColor=white)](#)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg?style=for-the-badge)](https://opensource.org/licenses/MIT)

[English](#) · [Русский](#)

</div>

---

<div align="center">
  
  <!-- ЗАМЕНИ ЭТУ ССЫЛКУ НА СВОЙ СКРИНШОТ -->
  <img src="https://github.com/ZigRain-hub/mimic-discord-easy/blob/main/assets/screenshot.png?raw=true" width="700" alt="QDCord Interface" />
  
  <br/>
  <sub>Интерфейс QDCord: нативный Rust + GDI+, без Electron и WebView</sub>
</div>

---

## ⚡ Почему QDCord?

Это **самостоятельная, облегчённая реализация** идеи [mimic-discord](https://github.com/Timon2306/mimic-discord). Вместо тяжеловесного стека (Tauri + React + WebView2) здесь используется **чистый Rust и Win32 API**.

*   **Один файл:** Никаких установщиков, зависимостей и `node_modules`.
*   **Прозрачность:** Всего 3 файла исходного кода. Аудит безопасности занимает один вечер.
*   **Изоляция:** Запускает сам себя с флагом `--qdcord-worker`, не скачивая сторонние программы.
*   **Атомарность:** Настройки и история пишутся через `.tmp` → `fsync` → `rename`. Никаких битых файлов при выключении ПК.

---

## 🚀 Что скачать (терминал не нужен)

👉 **[Скачать последнюю версию](../../releases/latest)** и откройте раздел **Assets**.

| Ваша система | Какой файл скачать |
| :--- | :--- |
| **Windows** (портативная) | `qdcord.exe` — скачал и запустил. Установка не нужна. |

> ⚠️ **Важно:** Положите файл `detectable-games.json` в ту же папку, где лежит `qdcord.exe`. Без него программа не увидит базу игр.
> 
> 🛡️ Не качайте `Source code (zip)`. Если Windows пишет «Система защитила ваш компьютер» — нажмите **Подробнее → Выполнить в любом случае** (программа не подписана сертификатом).

---

## ✨ Возможности

*   🏆 **Прокачка «Универсала»:** Параллельные батчи перебирают официальную базу Discord. Проверено: мифический значок открывается на 9 800+ играх.
*   ⏱️ **Умножение «Времени»:** 8 игр одновременно × 5 часов = 40 часов в статистике Discord.
*   ⏳ **Фарм часов:** Выбираете до 20 игр — они работают без смены, пока вы не нажмёте «Стоп» или не выйдет таймер.
*   🧹 **Без мусора:** Временный каталог удаляется сразу после каждого батча.
*   💾 **Умная история:** `history.json` помнит отработанные игры и пропускает их. Если файл повредится — он не потеряется, а переименуется в `history.broken-<unix>.json`.

---

## 🛠️ Для разработчиков

> **Стек:** Rust (Edition 2021), Win32 API, GDI+ (FFI), `std::process`, `serde`. Никаких Tauri, React или Node.js.

### Сборка из исходников

**Требования:** Rust (stable 1.75+) и MSVC toolchain (Visual Studio Build Tools с C++).

```powershell
# Клонируем репозиторий
git clone https://github.com/ZigRain-hub/mimic-discord-easy.git
cd mimic-discord-easy

# Собираем релиз
cargo build --release
