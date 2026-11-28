# Реверс-инжиниринг инсталлятора NVIDIA (`setup.exe` / `NVI2` / Extensions)

> Справочник для веток **1.x (`NVI2`)** и **2.x (`Beta` / нативный Rust-установщик)**. Все адреса, имена функций, флаги и ветки реестра получены прямым анализом бинарников драйвера **621.40 WHQL DCH** в **Ghidra 12.1.4**, **REA** и XML-манифестов (`.nvi` / `.cfg`).

---

## 1. `setup.exe` (x86 PE, `876 776` байт)

### 1.1 Проверка цифровой подписи `NVI2.dll`
- `FUN_00402420` загружает `NVI2\NVI2.DLL` через `FUN_00413bb0` -> `FUN_00423d80` -> `FUN_00423eb0`.
- Если активна проверка подписи (`*(char *)(param_1 + 0x25) != 0`), `FUN_00423eb0` проверяет статус службы `CryptSvc`, вызывает `CryptQueryObject` и `WinVerifyTrust` (`WINTRUST_ACTION_GENERIC_VERIFY_V2`).
- **Следствие для 1.x:** бинарный патч `NVI2.dll` без переподписи или одновременного патча `setup.exe` приводит к отказу загрузки `NVI2.dll` на целевой системе.

### 1.2 Принимаемые CLI-флаги
- **Унарные (`FUN_00404260`):** `-uninstall`, `-debug` (ожидание `IsDebuggerPresent` + `Sleep(250)`), `-log`, `-s` (`SilentInstall=1`), `-validate` (`ValidationInstall`), `-skipcritical`, `-allowDevicePackage` (`AllowDevicePackage=1`). Любой неизвестный `-Имя` устанавливает булево свойство `Имя=TRUE` на COM-объекте `NVI2.InstallPackagesManager`.
- **Ключ:Значение (`FUN_00403cc0`):**
  - `-deviceinitiated:...` (`FUN_00404fd0` выставляет `IgnorePnpFlag=1`, `IgnoreNonCriticalErrors=1`, `IgnoreReboot=1`, `IsSetupInitiated=0`);
  - `-exclude:...`, `-deselect:...`, `-log:...`, `-loglevel:...`, `-uiextension:...`;
  - `-ignoreConstraints:...` (`FUN_00403280` добавляет имена проверок в коллекцию `IgnoreConstraints`);
  - любой `-Ключ:Значение` передаётся в свойства `InstallPackagesManager`.
- **Позиционные аргументы (`FUN_00404630`):** имена устанавливаемых субпакетов.

---

## 2. `NVI2.dll` (x86 COM, `7 291 112` байт) и причина задержки старта

### 2.1 Опции `setup.cfg` (`<options>`)
Поддерживаемые опции: `clean`, `prestage`, `validate`, `validationFile`, `forcereboot`/`k`, `noreboot`/`n`, `passive` (`ProgressOnly`), `noeula` (`SkipEula`), `nofinish` (`SkipFinish`), `ignorepnp`, `progresswitheula`, `nosplash` (`NoSplashScreen`), `nvappinitiated`, `custominvokerid`, `enableTelemetry`. Внутри `NVI2.dll` также проверяется COM-свойство `SkipOptions` (`FUN_100fb020`).

### 2.2 Почему до `Stage Driver Package` проходит ~1.5–3 минуты
1. **Это НЕ проверка хэша `.NVH`:** в пакете `621.40` файл `.NVH` / `Hash.xml` отсутствует; `PerformChecksumValidationCallbackAction` (`FUN_100f28a0` -> `FUN_100f2c1c` -> `FUN_100f2f20`) запускает `PerformPackageFilesValidation` (`FUN_101d3390`) только если расширение явно возвращает `true`.
2. **`CNVInstaller::PerformStartupOperations` (`FUN_10045fb8`):** работа с `CEngineCache` — проверка дискового пространства и перенос/обновление движка в `C:\Program Files\NVIDIA Corporation\Installer2`.
3. **`CNVExtension::InternalLoad` (`FUN_10189af0`) и `InternalEvaluateBitness` (`FUN_1018ba60`):** так как `NVI2.dll` — **32-битный (`x86`)**, он отбрасывает `DisplayDriverExt64.dll` и загружает 32-битный `DisplayDriverExt.dll` (с проверкой пути/валидности `FUN_102df550`). Без него отваливается `ApplyShortlistedInfsToDevicePhases`.
4. **`CInstallationChecking` (`FUN_100f25d0`):**
   - вызывает расширения для `"initialization"` и `"Analyze"` (`FUN_100f3660`);
   - запускает `CheckConstraints` (`FUN_100f3c40` -> `FUN_100f3da0`): `SetupMissingFilesCheck`, `CNVDiskSpaceConstraint2::InvokeCheck` (`FUN_10174572` -> `FUN_10175250`, подсчёт размеров по INF/DriverStore/EngineCache), `CNVDeviceConstraint::InvokeCheck` (`FUN_10124540` -> `FUN_10124e50`, перебор `.inf` и сопоставление HWID через `SetupDi*`), кастомные проверки `DisplayDriverExt.dll` (`InfPlatformCheck`, `AllowInstallCheck`, `ExternalGraphicsDowngradeCheck`, `BlockTH1Install`, `CriticalBsodIssueCheck`);
   - вызывает `"after constraints check"` (`CDisplayDriverExtSite::AfterConstraintsCheck` -> `ShortlistApplicableInfs`).
5. **Объём INF в `Display.Driver\`:** в `621.40` содержится **43 `.inf` файла** общим размером **>50 МБ** (`nvlti.inf` — 14.8 МБ, `nvdmi.inf` — 6.8 МБ, `nvami.inf` — 4.2 МБ, `nvmii.inf` — 3.4 МБ и т.д.). Их разбор через `SetupOpenInfFileW` / `SetupDiBuildDriverInfoList` + `WinVerifyTrust` / `CryptCATAdminAcquireContext2` внутри `DisplayDriverExt.dll` и создаёт основную паузу перед установкой.
6. **Почему флаги не убирают паузу:** `-validate` включает режим валидации (`"Skipping device contraint for validation mode"`), но не ставит драйвер; `-ignoreConstraints:...` лишь помечает упавший constraint как `"Constraint skipped:"` внутри `CheckConstraints`, не отменяя инициализацию, кэш и `Analyze`.

---

## 3. UI и карусель презентаций (`NVI2UI.dll` + `presentations.*` + `progress.htm`)

- Форма прогресса в `NVI2\0000.ui.forms`:
  `<control style="progressPresentation" class="html" x="0pt" y="0pt" width="592pt" height="282pt" hidden="true" data="${{progressPresentationData}}" ready="@ContentReady" file="${{progressPresentationFile}}"/>`
- В `ProgressPresentationTemplate` стоит `hidden="false"`; `NVI2UI.dll` реализует `CHtmlSharedControlEventSink::OnLoad` и `CHtmlSharedControlScripting::put_ready`.
- **Важно:** в оригинальном `621.40` локальных слайдов игр нет — стоковый `NVI2\progress.htm` при старте запрашивает HTML по HTTPS с `https://gfwsl.geforce.com/nvidia_web_services/controller.driverinstallercontent.php?...` (`presentations.cfg`) и грузит его в `<iframe>`.
- Локальные `.jpg` (`NVI2\slide_*.jpg`) и обновлённый `NVI2\progress.htm` полностью совместимы со штатным `<manifest>` в `setup.cfg` (`<file name="${{ResourcesDirectory}}/*.jpg"/>`).

---

## 4. `PackageLauncher.exe` (7zSD SFX)

Конфиг-блок между PE-секциями и сигнатурой `7z\xBC\xAF\x27\x1C` (`1572` байта в `621.40`):
- `InstallPath=""`, `OverwriteMode="0"`, `RunProgram=".\\setup.exe"`, `SelfDelete="0"`, `GUIFlags="5612"`, `GUIMode="0"`, `Progress="yes"`.
- Для **2.x (`Beta`)** авто-распаковки без диалога выбора папки задаётся `InstallPath="C:\\NVIDIA\\OpenFrankenDriver\\<ver>"`, `GUIMode="1"`.

---

## 5. Детальная логика расширений и `.nvi`-фаз (база для инсталлятора 2.x)

### 5.1 `DisplayDriver.nvi` + `DisplayDriverExt.dll` (`FUN_1000c400` и др.)
- **В DCH (`isDeclarative`):** ядро (`nvlddmkm.sys`, DX/OGL/CUDA/Vulkan ICD `nv-vk64.json`, OpenCL, NVAPI, `NVDisplay.ContainerLocalSystem`, `NVSMI`, базовый `NGXPath`) устанавливает сам `nv_dispi.inf` через Windows PnP.
- **Обязательные фазы сверх `nv_dispi.inf`:**
  1. **`NGXCore` и `nvngx_update.exe` (`invokeNGXUpdater`, `FUN_1000c400`):**
     - перед установкой удаляются старые `HKLM\SOFTWARE\NVIDIA Corporation\Global\NGXCore` (`FullPath`, `Installed`, `Version`);
     - после PnP-установки определяется путь пакета в `DriverStore`, скрыто запускается `<DriverStore>\nvngx_update.exe -forced_update` (`CREATE_NO_WINDOW`, ожидание завершения);
     - записываются `HKLM\SOFTWARE\NVIDIA Corporation\Global\NGXCore`: `FullPath = <DriverStore>` (`REG_SZ`) и `Installed = 1` (`REG_DWORD`).
  2. **MSI-прерывания (`CheckDriverInterruptModeIssue` -> `FUN_100495d0`):**
     - проверяет `RMIntrDisableMsixOnNextReload` в `Device Parameters` устройства `PCI\VEN_10DE`;
     - если значение `!= 0` и `Interrupt Management\MessageSignaledInterruptProperties\MSISupported == 1`, сбрасывает `MSISupported = 0` и перезапускает устройство (`RmInterruptModeIssue`).
  3. **Висящие виртуальные устройства (`CheckDanglingDeviceRemoval`):** удаляет `ROOT\NVMODULETRACKER` и `ROOT\NVVHCI` при наличии.
  4. **Чистка кэшей и legacy-хвостов:** `DXCache` / `GLCache` / `ComputeCache`, устаревшие `Program Files\NVIDIA Corporation\DRS` (`dbInstaller.exe`, `nvdrsdb.bin`), `nvinit*.dll`, `nvumdshim*.dll`, `nvoglshim*.dll`, `coprocmanager\*`, `nvgsync\*`, legacy `opencl.dll`, а также `HKLM\SOFTWARE\NVIDIA Corporation\Global\Vulkan` (`FUN_1009c7e0` / `FUN_1009cd5e`).
  5. **Store NVCPL AppX vs Win32 `Display.NVCPL`:** удаление конфликтующего AppX `*NVIDIACorp.NVIDIAControlPanel*` и ключа `InstalledPfns\NVIDIACorp.NVIDIAControlPanel_56jybvy8sckqj`.
  6. **Подтверждение и ARP:** проверка наличия `System32\nvinfo.pb` и `System32\nvapi64.dll` (`Confirm-install`), очистка старых ARP-записей (`RemoveOldARPEntry`, `FUN_100c65e0` — `"NVIDIA Drivers"` / `nvdisp*.nvu`).
  7. **`CreateDRSSetting` (`FUN_10041ff0`):** пишет `DRS_00380880` **только** при наличии свойства `EnableWaterMarking`; для пользовательских драйверов не применяется.

### 5.2 `PhysX.nvi` + `PhysXExt.dll` (`FUN_10004150`)
- У `PhysX` нет `.inf`.
- Перед установкой `PhysXExt.dll` (`uninstallOldMSI`) ищет MSI по `UpgradeCodes` и тихо удаляет старые версии (`MsiSetInternalUI` + `MsiConfigureProductExW`).
- Копируются файлы в `C:\Program Files (x86)\NVIDIA Corporation\PhysX\Common` и `\Engine\...`.
- **Критично (2 ветки реестра):** из-за 32-битного `NVI2` фаза `addRegistries` (`platform="x86"`) пишет в **`HKLM\SOFTWARE\WOW6432Node\AGEIA Technologies`** и **`HKLM\SOFTWARE\WOW6432Node\NVIDIA Corporation\{PhysX_SystemSoftware,PhysXUpdateLoader}`**, а `addRegistries64` — в нативный 64-битный **`HKLM\SOFTWARE\AGEIA Technologies`** (`PhysX_A64_Engines`) и **`HKLM\SOFTWARE\NVIDIA Corporation\PhysXUpdateLoader\DllTable\351C9090-...`**.
- В конец системного `PATH` добавляется `C:\Program Files (x86)\NVIDIA Corporation\PhysX\Common`.

### 5.3 `HDAudio.nvi` + `HDAudioExt.dll` (`FUN_10004750`)
- На x64-десктопах устанавливается обычный `HDAudio\nvhda.inf` (плюс `removePhantoms` для аудиоустройств). `UninstallExtensionInfs` используется только на `arm64`.
- `MSVCRT` в дистрибутиве содержит оба раннейма: `MSVCRT\vc_redist.x64.exe` и `MSVCRT\vc_redist.x86.exe`.

---

## 6. Маппинг реверса в нативный установщик 2.x (`sfx_rust` и `setup_rust`)

| Оригинальный компонент NVIDIA | Замена в ветке 2.x (`--beta-installer`) | Ключевые особенности реализации |
|---|---|---|
| `PackageLauncher.exe` (`7zSD` SFX) | `src/sfx_rust` -> `sfx_stub.exe` | Безрамочный UI Variant 03E (`520×156`, Phosphor Console `#0E1610`, 4px полоса `#76B900`, `[OK]`/`[..]`, автоцентрирование и очистка папки при отмене); конфиг-блок `;!@OFD@!1`..`;!@OFD_END@!`; декодер `sevenz-turbo 0.31.0` + `SliceReadAt` (игнорирует хвост подписи `signtool`); `clear_readonly_if_exists`; проброс `--silent` в `setup.exe`. |
| `setup.exe` + `NVI2.dll` + `NVI2UI.dll` | `src/setup_rust` -> `setup.exe` | Нативный Win32 GDI GUI (3 этапа, живой лог в `C:\ProgramData\OpenFrankenDriver\setup.log`, 17-слайдовая локальная витрина `Showcase/` без обращения к `gfwsl.geforce.com`). Поддерживает `--uninstall`, `--silent`, `--clean`, `--nvpi`. |
| `DisplayDriverExt.dll` (`invokeNGXUpdater`, MSI, чистки) | `src/setup_rust/src/installer.rs` | SetupAPI-установка `nv_dispi.inf` (без сканирования 43 ноутбучных `.inf`), запуск `<DriverStore>\nvngx_update.exe -forced_update` и запись `NGXCore`, снятие старых конфликтов, импорт `FrankenDriver.cer`, интеграция `Display.NVCPL` и `NVProfileInspector`. |
| `PhysXExt.dll` + `PhysX.nvi` | `physx_manifest.rs` + `beta_builder.generate_physx_manifest` | Экстракт метаданных `PhysX` в `PhysX\physx_manifest.json` на этапе сборки и запись в обе ветки реестра (`WOW6432Node` и нативный 64-bit `HKLM\SOFTWARE\AGEIA Technologies`). |
| `HDAudioExt.dll` + `HDAudio.nvi` | `src/setup_rust/src/installer.rs` | Штатная установка `HDAudio\nvhda.inf` при выбранном компоненте аудиодрайвера. |
| Ручной аудит изменений `.nvi` при обновлении ветки | `beta_builder.check_nvi_phases_guard` | Автоматический белый список всех фаз `.nvi`: останавливает сборку (`RuntimeError`, при необходимости обходится флагом `--ignore-phase-guard`), если в новом драйвере NVIDIA появилась неизвестная фаза. |
