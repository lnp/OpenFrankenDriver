# Руководство: как модифицировать оригинальный драйвер NVIDIA

Данное руководство подробно описывает процесс модификации любого официального драйвера NVIDIA для поддержки десктопных видеокарт-мутантов (RTX 3070m, RTX 3060m и др.).

---

## 1. Подготовка окружения

Для сборки модифицированного пакета потребуются:
- **Операционная система**: Windows 10 или Windows 11 (64-bit).
- **Python**: версия 3.10 или выше. Установите зависимости:
  ```cmd
  pip install pefile Pillow
  ```
- **7-Zip**: архиватор, установленный по стандартному пути (`C:\Program Files\7-Zip\7z.exe`).
- **Windows SDK**: необходим для системных утилит `makecat.exe` и `signtool.exe` (входит в состав Windows 10/11 SDK или Visual Studio).
- **Rust toolchain (`rustc` / `cargo` MSVC и Windows SDK)**: необходим **только** при сборке нативного установщика 2.x с флагом `--beta-installer` (проверено на `rustc 1.99.1`).
- **Исходный чистый драйвер NVIDIA**: официальный установщик DCH WHQL с [сайта NVIDIA](https://www.nvidia.com/Download/index.aspx).

---

## 2. Способ А: Автоматическая сборка скриптом (Быстро и надёжно)

В репозитории подготовлен мастер-скрипт `scripts/build.py`, который выполняет все операции автоматически в один клик.

### Команда запуска:
```cmd
python scripts/build.py --nvidia-exe "C:\Path\To\621.40-desktop-win10-win11-64bit-international-dch-whql.exe"
```

### Дополнительные флаги (при необходимости):
- `--beta-installer` — сборка **OpenFrankenDriver v2 Beta** (`OpenFrankenDriver_v2_Beta_<ver>.exe`): полностью заменяет проприетарный `PackageLauncher.exe` и `NVI2` открытыми нативными Rust-компонентами [`src/sfx_rust`](../src/sfx_rust) (`sfx_stub.exe`, Variant `03E` + `sevenz-turbo`) и [`src/setup_rust`](../src/setup_rust) (`setup.exe`), вырезает bloat-компоненты и добавляет опциональный `NVProfileInspector`.
- `--ignore-phase-guard` — продолжить сборку `--beta-installer`, даже если **Phase Guard** обнаружил в новом драйвере NVIDIA неизвестную `.nvi`-фазу.
- `--output "C:\Path\To\MyDriver.exe"` — путь и имя выходного готового `.exe`.
- `--pfx "C:\Path\To\cert.pfx"` — путь к существующему сертификату подписи. Если флаг опущен и локальный ключ отсутствует, скрипт автоматически сгенерирует доверенный сертификат через `scripts/create_cert.ps1`.
- `--pfx-pass "1234"` — пароль от PFX-сертификата (по умолчанию `1234`).
- `--slim` — облегчённая сборка ветки 1.x: вырезает NVIDIA App / Overlay / ShadowPlay / FrameViewSDK / телеметрию, сохраняя `Display.Driver`, Win32-панель управления, `PhysX`, `HDAudio`, `MSVCRuntime` и файлы лицензии.
- `--keep-telemetry` — оставить фоновый плагин `_nvtopps` активным (не прописывать `DisableLoad=1`).

Скрипт самостоятельно распакует архив, динамически определит секции в `nv_dispi.inf`, сгенерирует рекурсивный каталог `nv_disp.cat`, подпишет его со штампом времени DigiCert RFC3161, при наличии MSVC/GCC пересоберёт `install_helper.c` (иначе использует готовый), интегрирует Win32 Панель управления, выполнит встроенный **Smoke Check** (а в режиме `--beta-installer` — ещё и **Phase Guard** `.nvi` и сборку `setup.exe` / `sfx_stub.exe`) и упакует монолитный самораспаковывающийся инсталлятор (`-mx=4`).

### Тестирование нативных компонентов (для `--beta-installer`):
```powershell
# Юнит-тесты установщика setup_rust и SFX-распаковщика sfx_rust
cargo test --manifest-path "src\setup_rust\Cargo.toml"
cargo test --manifest-path "src\sfx_rust\Cargo.toml"

# Полная проверка потоковой распаковки собранного монолитного .exe и проверка подписи
cargo test --release --manifest-path "src\sfx_rust\Cargo.toml" verify_real_monolithic_exe_extracts -- --ignored --nocapture
signtool verify /pa "C:\drivers\OpenFrankenDriver_v2_Beta_621.40.exe"
```

---

## 3. Способ Б: Ручная модификация шаг за шагом

Если вы хотите выполнить каждый шаг вручную или понять техническую суть каждого действия:

### Шаг 1: Распаковка оригинального пакета
1. Нажмите правой кнопкой мыши по скачанному `.exe` от NVIDIA -> `7-Zip` -> `Распаковать в...`.
2. В распакованной папке удалите каталог `NVPCF` (драйвер питания ноутбука, вызывающий конфликты на десктопе).
3. Проверьте папку `Display.Driver\ngx-builtins\`: если там лежит сжатый файл `nvngx_dlssd.dl_`, распакуйте его системной командой:
   ```cmd
   expand nvngx_dlssd.dl_ nvngx_dlssd.dll
   ```

### Шаг 2: Внедрение идентификаторов устройств в `nv_dispi.inf`
Откройте файл `Display.Driver\nv_dispi.inf` в кодировке ANSI/Windows-1251 (или Latin1) и внесите следующие изменения:

1. **Секция Windows 10 / Windows 11 (`17134`)**:
   Найдите строку `[NVIDIA_Devices.NTamd64.10.0...17134]` и сразу под ней вставьте строки ваших моделей из файла `data/devices.json` (список `devices_17134`).
2. **Секция актуальных сборок Windows 11 (`26200`)**:
   Найдите строку `[NVIDIA_Devices.NTamd64.10.0...26200]` и сразу под ней вставьте строки ваших моделей из файла `data/devices.json` (список `devices_26200`).
3. **Обход Ошибки 43 (Error 43)**:
   Найдите секции:
   - `[nv_miscBase_addreg__01]`
   - `[nv_miscBase_addreg__05]`
   - `[nv_miscBase_addreg__09]`
   - `[nv_miscBase_addreg__10]`
   - `[nv_miscBase_addreg__16]`
   В конец каждой из этих секций добавьте параметр реестра:
   ```ini
   HKR,,RM1457588,%REG_DWORD%,1
   ```
4. **Строковые названия моделей**:
   В конец секции `[Strings]` добавьте определения названий моделей (список `strings` из `data/devices.json`).
5. Сохраните `nv_dispi.inf`.

### Шаг 3: Генерация и подписание каталога безопасности
Так как `nv_dispi.inf` был изменён, старый каталог `nv_disp.cat` более недействителен.
1. Создайте в папке `Display.Driver` файл `nv_disp.cdf` со списком всех файлов каталога:
   ```ini
   [CatalogHeader]
   Name=nv_disp.cat
   ResultDir=.

   [CatalogFiles]
   <HASH>file1.dll=file1.dll
   ...
   ```
2. Сгенерируйте каталог:
   ```cmd
   makecat -v nv_disp.cdf
   ```
3. Подпишите каталог вашим сертификатом со штампом времени RFC3161:
   ```cmd
   signtool sign /f "FrankenDriver.pfx" /p 1234 /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 nv_disp.cat
   ```

### Шаг 4: Подключение автоматической установки сертификата
1. Создайте в корне инсталлятора папку `FrankenDriver`.
2. Поместите туда публичный сертификат `FrankenDriver.cer` и файл `src/FrankenDriver.nvi`.
3. В файле `setup.cfg` подключите зависимость:
   ```xml
   <sub-package name="FrankenDriverCert" disposition="default" hidden="true" />
   ```
   и укажите `FrankenDriverCert` в блоке `<dependencies>` субпакета `Display.Driver` (`type="after"`).

### Шаг 5: Установка драйвера
Запустите `setup.exe`. Установщик штатно установит драйвер на видеокарту.
