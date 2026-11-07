<div align="center">

[![Русский](https://img.shields.io/badge/Язык-Русский-1f883d?style=for-the-badge)](#)
[![English](https://img.shields.io/badge/Language-English-0969da?style=for-the-badge)](README_EN.md)

<br>

<a href="https://github.com/lnp/OpenFrankenDriver"><img src="assets/header.png" alt="OpenFrankenDriver" width="840"></a>
<br>

**Полностью открытый, бесплатный и безопасный драйвер NVIDIA для китайских видеокарт-мутантов**  
*(RTX 4070m, RTX 4060m, RTX 3070m, RTX 3060m, RTX 2060m, GTX 1660m и ещё 65 моделей на базе мобильных чипов AD106/GA104/GA106/TU106)*

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![NVIDIA Driver Version](https://img.shields.io/badge/NVIDIA-621.14%20WHQL-76B900?logo=nvidia)](https://www.nvidia.com)
[![Anti-Cheat Ready](https://img.shields.io/badge/Anti--Cheat-EAC%20%7C%20BattlEye%20%7C%20VAC%20Passed-success)](docs/ARCHITECTURE.md)
[![Platform](https://img.shields.io/badge/Windows-10%20%7C%2011%20(24H2%2B)-0078D6?logo=windows)](docs/SUPPORTED_DEVICES.md)
[![Boosty](https://img.shields.io/badge/Boosty-Поддержать%20проект-F15F2C?logo=boosty)](https://boosty.to/defrag)

---

### 📥 [Скачать готовый драйвер OpenFrankenDriver 621.14 (Google Drive)](https://drive.google.com/file/d/1uIS2bM4CIcCxQ9KAcM-kB_A9dio3UTHF/view?usp=sharing)
*(Размер: ~1.07 ГБ • Стабильная ветка 621.14 WHQL • Встроена автономная классическая Панель управления)*

---

### Поддержка проекта
Проект развивается на чистом энтузиазме. Драйверы и скрипты сборки всегда будут полностью открытыми и бесплатными. 

Если сборка сэкономила вам время, оживила вашу видеокарту и избавила от танцев с бубном — вы можете добровольно поддержать разработку на [Boosty](https://boosty.to/defrag).

---

[**Поддерживаемые видеокарты**](docs/SUPPORTED_DEVICES.md) • [**Инструкция по модификации**](#-как-модифицировать-оригинальный-драйвер-инструкция) • [**Архитектура**](docs/ARCHITECTURE.md) • [**Решение проблем**](docs/TROUBLESHOOTING.md)

</div>

---

## 🚀 Почему OpenFrankenDriver? (Преимущества перед оригиналом)

Оригинальный проект **FrankenDriver** (`arutar/FrankenDriver`) долгое время был популярным решением для запуска китайских карт с мобильными чипами. Однако оригинальный репозиторий **не является open-source** (в нём нет исходного кода и скриптов), его бесплатные версии давно устарели, а свежие сборки распространяются по платной подписке на Boosty.

**OpenFrankenDriver** — открытая, бесплатная и технически доработанная альтернатива оригинальному проекту:

| Возможность / Параметр | Оригинальный FrankenDriver | <img src="assets/badge.png" alt="OpenFrankenDriver" width="180"> |
|---|---|---|
| **Открытость и прозрачность** | ❌ Закрытый бинарник. Репозиторий пуст, исходного кода нет. | ✅ **100% Open-Source**. Весь пайплайн сборки и исходники открыты. |
| **Доступность версий** | ❌ Свежие версии закрыты за платным пейволлом. | ✅ **Всегда бесплатно**. Актуальная стабильная ветка **621.14 WHQL**. |
| **Установка в 1 клик (Zero-Touch)** | ❌ Красное окно Windows Security («Издатель не проверен»). Требовалась ручная установка сертификата. | ✅ **Тихая автоматическая регистрация**. Сертификат сам импортируется в `Root` и `TrustedPublisher`. Никаких красных окон. |
| **Классическая Панель управления** | ❌ На чистой Windows или лёгких сборках панель отсутствует (требовался Microsoft Store и интернет). | ✅ **Встроена автономная Win32-панель** (`nvcplui.exe`). Работает из коробки на Windows LTSC и Compact без Store и без сети. |
| **Безопасность и античиты** | ❓ Закрытые бинарники вызывали вопросы у пользователей. | ✅ **Оригинальные нетронутые ядра NVIDIA**. 100% WHQL-целостность, чистый тест `signtool`, `testsigning No`, полная работа EAC / BattlEye / CS2. |

---

## 💾 Быстрая установка (Для пользователей)

1. **Скачайте готовый установщик** по ссылке в шапке или с [Google Drive](https://drive.google.com/file/d/1uIS2bM4CIcCxQ9KAcM-kB_A9dio3UTHF/view?usp=sharing).
2. *(Рекомендуется)* Если вы обновляетесь со старых версий или меняли видеокарту, удалите предыдущий драйвер через **DDU (Display Driver Uninstaller)** в безопасном режиме.
3. Запустите скачанный `.exe` файл.
4. Выберите **«Графический драйвер NVIDIA»** и нажмите **«Далее»**.
5. По завершении установки видеокарта определится без «Ошибки 43», а на рабочем столе и в контекстном меню сразу появится рабочая **Панель управления NVIDIA**.

---

## 🛠️ Как модифицировать оригинальный драйвер (Инструкция)

Вы можете в любой момент взять любой официальный чистый драйвер NVIDIA с официального сайта и самостоятельно получить модифицированный драйвер OpenFrankenDriver.

### Способ 1: Автоматическая сборка скриптом (Рекомендуется)

1. **Установите необходимые компоненты**:
   - Python 3.10+ (установите модуль `pefile`: `pip install pefile`)
   - Архиватор 7-Zip (`C:\Program Files\7-Zip\7z.exe`)
   - Windows 10/11 SDK (необходим для системных утилит `makecat.exe` и `signtool.exe`)
2. **Скачайте официальный драйвер NVIDIA**:
   - Перейдите на [официальный сайт NVIDIA](https://www.nvidia.com/Download/index.aspx) и скачайте стандартный DCH WHQL драйвер (например, `621.14-desktop-win10-win11-64bit-international-dch-whql.exe`).
3. **Запустите сборку одной командой**:
   ```cmd
   python scripts/build.py --nvidia-exe "C:\путь\к\скачанному_файлу_nvidia.exe"
   ```
4. Скрипт в автоматическом режиме:
   - Распакует официальный установщик;
   - Внедрит идентификаторы 71 видеокарты-мутанта и параметр обхода Error 43 в `nv_dispi.inf`;
   - Сформирует полный каталог безопасности `nv_disp.cat` (271 файл) и подпишет его доверенным сертификатом со штампом времени DigiCert;
   - Интегрирует классическую автономную панель управления NVIDIA (`nvcplui.exe`);
   - Упакует монолитный самораспаковывающийся `.exe` установщик без ошибки `0xc0000005`.

---

### Способ 2: Ручная пошаговая модификация

Если вы хотите выполнить модификацию вручную без автоматического скрипта:

1. **Распаковка**:
   - Распакуйте официальный установщик NVIDIA через 7-Zip в отдельную папку (например, `Driver_Unpacked`).
   - Удалите папку `NVPCF` (драйвер батареи ноутбука, не нужен на десктопе).
   - Если в папке `Display.Driver\ngx-builtins\` файл `nvngx_dlssd.dl_` сжат, распакуйте его командой:  
     `expand nvngx_dlssd.dl_ nvngx_dlssd.dll`.

2. **Модификация INF-файла (`Display.Driver\nv_dispi.inf`)**:
   - Откройте `Display.Driver\nv_dispi.inf` в текстовом редакторе.
   - Найдите секции:
     - `[NVIDIA_Devices.NTamd64.10.0...17134]` — добавьте строки поддерживаемых устройств для Windows 10/11 (список доступен в `data/devices.json`).
     - `[NVIDIA_Devices.NTamd64.10.0...26200]` — добавьте строки поддерживаемых устройств для Windows 11 24H2 (список доступен в `data/devices.json`).
   - В секциях параметров реестра `[nv_miscBase_addreg__01]`, `[nv_miscBase_addreg__05]`, `[nv_miscBase_addreg__09]`, `[nv_miscBase_addreg__10]`, `[nv_miscBase_addreg__16]` добавьте строку обхода Ошибки 43:
     ```ini
     HKR,,RM1457588,%REG_DWORD%,1
     ```
   - В конец секции `[Strings]` добавьте названия моделей видеокарт (из `data/devices.json`).

3. **Создание и подписание каталога безопасности (`nv_disp.cat`)**:
   - Создайте CDF-файл со списком всех 271 файлов папки `Display.Driver`:
     `makecat -v nv_disp.cdf`
   - Подпишите созданный `nv_disp.cat` через `signtool.exe`:
     `signtool sign /f your_cert.pfx /p 1234 /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 nv_disp.cat`
   - Скопируйте готовый `nv_disp.cat` также в папку `EACSupport\nv_disp.cat`.

4. **Интеграция сертификата**:
   - Поместите открытый сертификат `FrankenDriver.cer` и файл `src/FrankenDriver.nvi` в папку `FrankenDriver\`.
   - В `setup.cfg` подключите субпакет `FrankenDriverCert` как зависимость перед `Display.Driver`.

5. **Запуск установки**:
   - Запустите `setup.exe` из папки `Driver_Unpacked` — установка пройдёт в штатном режиме.

Подробное описание параметров и архитектуры см. в [**docs/HOW_TO_BUILD.md**](docs/HOW_TO_BUILD.md).

---

## 🛡️ Безопасность: не сгорит ли видеокарта? Не забанят ли в играх?

1. **Видеокарта и ПК в полной безопасности**:
   - Драйвер не меняет напряжение (вольтаж), алгоритмы питания или прошивку (VBIOS) видеокарты.
   - Драйвер лишь сообщает Windows: *«Разреши этому кристаллу запуститься на шине PCIe x16 и не выбрасывай ошибку 43»*.
   - Все системы температурного троттлинга, управления кулерами и лимитов TDP продолжают управляться штатным контроллером самой видеокарты.
2. **Античиты и онлайн-игры (EAC, BattlEye, VAC, FaceIT)**:
   - Файлы ядра (`nvlddmkm.sys`, компоненты DirectX, Vulkan, CUDA) **не патчатся и не взламываются**.
   - На них сохранена официальная цифровая подпись Microsoft WHQL.
   - Тестовый режим Windows (`testsigning`) остаётся **выключенным**. Античиты видят подлинные системные бинарники и не выдают блокировок.

---

## 🧩 Поддерживаемые видеокарты (71 модель / конфигурация)

Драйвер поддерживает все десктопные видеокарты-мутанты на базе мобильных чипов NVIDIA серии GeForce RTX 40, RTX 30, RTX 20, GTX 16 и профессиональных мобильных карт RTX / Quadro.

**Популярные поддерживаемые карты:**
- **NVIDIA GeForce RTX 4070 Laptop GPU / RTX 4070m** (`DEV_2820`, `DEV_2860`)
- **NVIDIA GeForce RTX 4060 Laptop GPU / RTX 4060m** (`DEV_28A0`)
- **NVIDIA GeForce RTX 4050 Laptop GPU / RTX 4050m** (`DEV_28A1`)
- **NVIDIA GeForce RTX 3070 Laptop GPU / RTX 3070m** (`DEV_249D`, `DEV_249C`)
- **NVIDIA GeForce RTX 3060 Laptop GPU / RTX 3060m (6GB & 12GB)** (`DEV_2520`, `DEV_2521`, `DEV_2560`)
- **NVIDIA RTX A3000 Laptop GPU / RTX A3000m** (`DEV_24B8`)
- **NVIDIA RTX A4000 Laptop GPU / RTX A4000m** (`DEV_24B7`)
- **NVIDIA RTX A5000 Laptop GPU / RTX A5000m** (`DEV_24B6`)
- **NVIDIA GeForce RTX 3080 Laptop GPU / RTX 3080m** (`DEV_2484`, `DEV_249C`)
- **NVIDIA GeForce RTX 3050 Laptop GPU / RTX 3050m** (`DEV_25A2`, `DEV_25E2`)
- **NVIDIA GeForce RTX 2060 Laptop GPU / RTX 2060m** (`DEV_1F11`, `DEV_1F15`, `DEV_1F51`)
- **NVIDIA GeForce RTX 2070 Laptop GPU / RTX 2070m** (`DEV_1F10`, `DEV_1F14`, `DEV_1F50`)
- **NVIDIA GeForce RTX 2080 Laptop GPU / RTX 2080m** (`DEV_1E90`, `DEV_1ED0`)
- **NVIDIA GeForce GTX 1660 Ti Laptop GPU / GTX 1660m** (`DEV_2191`)
- **NVIDIA GeForce GTX 1650 Laptop GPU / GTX 1650m** (`DEV_1F99`, `DEV_1F9D`)

👉 [**Ознакомиться с полным списком всех 71 конфигураций и их аппаратными ID**](docs/SUPPORTED_DEVICES.md)

---

## 📚 Дополнительная документация

- [**docs/HOW_TO_BUILD.md**](docs/HOW_TO_BUILD.md) — Исчерпывающее пошаговое руководство по сборке и модификации.
- [**docs/SUPPORTED_DEVICES.md**](docs/SUPPORTED_DEVICES.md) — Полная таблица со всеми 71 устройствами и их PCI Hardware ID.
- [**docs/ARCHITECTURE.md**](docs/ARCHITECTURE.md) — Глубокий технический разбор: NVI2, Win32 Control Panel, PE-структура и хэши каталогов.
- [**docs/TROUBLESHOOTING.md**](docs/TROUBLESHOOTING.md) — Решение возможных проблем, правильная очистка через DDU и проверка параметров через `nvidia-smi`.

---

## 📜 Лицензия и отказ от ответственности

- Проект распространяется под открытой лицензией [MIT](LICENSE).
- Торговые марки NVIDIA, GeForce, RTX, GTX являются собственностью корпорации NVIDIA.
- Данный проект не аффилирован с корпорацией NVIDIA и создан сообществом энтузиастов исключительно для обеспечения совместимости и продления срока службы оборудования.
