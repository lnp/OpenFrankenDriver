#include <windows.h>
#include <shlobj.h>
#include <stdio.h>

void LogMsg(const wchar_t* msg) {
    wchar_t tempPath[MAX_PATH];
    GetTempPathW(MAX_PATH, tempPath);
    wcscat_s(tempPath, MAX_PATH, L"nvcpl_install.log");
    FILE* f = NULL;
    if (_wfopen_s(&f, tempPath, L"a, ccs=UTF-8") == 0 && f) {
        SYSTEMTIME st;
        GetLocalTime(&st);
        fwprintf(f, L"[%04d-%02d-%02d %02d:%02d:%02d] %s\n", 
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, msg);
        fclose(f);
    }
}

HRESULT CreateCleanShortcut(const wchar_t* lnkPath, const wchar_t* targetPath, const wchar_t* args, const wchar_t* workDir, const wchar_t* iconPath, int iconIndex, const wchar_t* desc) {
    HRESULT hr = CoInitialize(NULL);
    IShellLinkW* psl = NULL;
    hr = CoCreateInstance(&CLSID_ShellLink, NULL, CLSCTX_INPROC_SERVER, &IID_IShellLinkW, (void**)&psl);
    if (SUCCEEDED(hr)) {
        psl->lpVtbl->SetPath(psl, targetPath);
        if (args && wcslen(args) > 0) psl->lpVtbl->SetArguments(psl, args);
        if (workDir && wcslen(workDir) > 0) psl->lpVtbl->SetWorkingDirectory(psl, workDir);
        if (desc && wcslen(desc) > 0) psl->lpVtbl->SetDescription(psl, desc);
        if (iconPath && wcslen(iconPath) > 0) {
            psl->lpVtbl->SetIconLocation(psl, iconPath, iconIndex);
        } else {
            psl->lpVtbl->SetIconLocation(psl, targetPath, iconIndex);
        }

        IPersistFile* ppf = NULL;
        hr = psl->lpVtbl->QueryInterface(psl, &IID_IPersistFile, (void**)&ppf);
        if (SUCCEEDED(hr)) {
            DeleteFileW(lnkPath);
            hr = ppf->lpVtbl->Save(ppf, lnkPath, TRUE);
            ppf->lpVtbl->Release(ppf);
        }
        psl->lpVtbl->Release(psl);
    }
    CoUninitialize();
    return hr;
}

void RemoveEmptyDirectory(const wchar_t* dir) {
    if (!dir || wcslen(dir) == 0) return;
    RemoveDirectoryW(dir);
}

void DeleteRegKeyRecursive(HKEY hRootKey, const wchar_t* strSubKey) {
    HKEY hKey;
    if (RegOpenKeyExW(hRootKey, strSubKey, 0, KEY_READ | KEY_WRITE, &hKey) == ERROR_SUCCESS) {
        DWORD dwSize = MAX_PATH;
        wchar_t achKey[MAX_PATH];
        while (RegEnumKeyExW(hKey, 0, achKey, &dwSize, NULL, NULL, NULL, NULL) == ERROR_SUCCESS) {
            DeleteRegKeyRecursive(hKey, achKey);
            dwSize = MAX_PATH;
        }
        RegCloseKey(hKey);
    }
    RegDeleteKeyW(hRootKey, strSubKey);
}

void CleanDirectoryFiles(const wchar_t* dirPath) {
    wchar_t searchPath[MAX_PATH];
    swprintf_s(searchPath, MAX_PATH, L"%s\\*.*", dirPath);
    WIN32_FIND_DATAW fd;
    HANDLE hFind = FindFirstFileW(searchPath, &fd);
    if (hFind != INVALID_HANDLE_VALUE) {
        do {
            if (!(fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)) {
                wchar_t filePath[MAX_PATH];
                swprintf_s(filePath, MAX_PATH, L"%s\\%s", dirPath, fd.cFileName);
                DeleteFileW(filePath);
            }
        } while (FindNextFileW(hFind, &fd));
        FindClose(hFind);
    }
}

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE hPrevInstance, PWSTR pCmdLine, int nCmdShow) {
    // Determine 64-bit Program Files
    wchar_t dstDir[MAX_PATH];
    DWORD len = GetEnvironmentVariableW(L"ProgramW6432", dstDir, MAX_PATH);
    if (len == 0 || len >= MAX_PATH) {
        SHGetFolderPathW(NULL, CSIDL_PROGRAM_FILES, NULL, 0, dstDir);
    }
    wcscat_s(dstDir, MAX_PATH, L"\\NVIDIA Corporation\\Control Panel Client\\");

    wchar_t logBuf[1024];

    // =========================================================================
    // UNINSTALL MODE
    // =========================================================================
    if (pCmdLine && (wcsstr(pCmdLine, L"/uninstall") || wcsstr(pCmdLine, L"-uninstall") || wcsstr(pCmdLine, L"/u"))) {
        LogMsg(L"install_helper: UNINSTALL mode started.");

        // 1. Delete Desktop Shortcuts
        int csidlsDesktop[] = { CSIDL_COMMON_DESKTOPDIRECTORY, CSIDL_DESKTOPDIRECTORY };
        for (int i = 0; i < 2; i++) {
            wchar_t deskPath[MAX_PATH];
            if (SUCCEEDED(SHGetFolderPathW(NULL, csidlsDesktop[i], NULL, 0, deskPath))) {
                wchar_t search[MAX_PATH];
                swprintf_s(search, MAX_PATH, L"%s\\*.lnk", deskPath);
                WIN32_FIND_DATAW fd;
                HANDLE hFind = FindFirstFileW(search, &fd);
                if (hFind != INVALID_HANDLE_VALUE) {
                    do {
                        if (wcsstr(fd.cFileName, L"NVIDIA Control") || wcsstr(fd.cFileName, L"NVCPL") || 
                            wcsstr(fd.cFileName, L"Панель") || wcsstr(fd.cFileName, L"Рџ")) {
                            wchar_t lnk[MAX_PATH];
                            swprintf_s(lnk, MAX_PATH, L"%s\\%s", deskPath, fd.cFileName);
                            DeleteFileW(lnk);
                            SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, lnk, NULL);
                        }
                    } while (FindNextFileW(hFind, &fd));
                    FindClose(hFind);
                }
            }
        }

        // 2. Delete Start Menu Shortcuts
        int csidlsPrograms[] = { CSIDL_COMMON_PROGRAMS, CSIDL_PROGRAMS };
        for (int i = 0; i < 2; i++) {
            wchar_t startPath[MAX_PATH];
            if (SUCCEEDED(SHGetFolderPathW(NULL, csidlsPrograms[i], NULL, 0, startPath))) {
                wchar_t nvdDir[MAX_PATH];
                swprintf_s(nvdDir, MAX_PATH, L"%s\\NVIDIA Corporation", startPath);
                
                wchar_t search[MAX_PATH];
                swprintf_s(search, MAX_PATH, L"%s\\*.lnk", nvdDir);
                WIN32_FIND_DATAW fd;
                HANDLE hFind = FindFirstFileW(search, &fd);
                if (hFind != INVALID_HANDLE_VALUE) {
                    do {
                        if (wcsstr(fd.cFileName, L"NVIDIA Control") || wcsstr(fd.cFileName, L"Панель") || 
                            wcsstr(fd.cFileName, L"Удалить") || wcsstr(fd.cFileName, L"Рџ") || wcsstr(fd.cFileName, L"РЈ")) {
                            wchar_t lnk[MAX_PATH];
                            swprintf_s(lnk, MAX_PATH, L"%s\\%s", nvdDir, fd.cFileName);
                            DeleteFileW(lnk);
                            SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, lnk, NULL);
                        }
                    } while (FindNextFileW(hFind, &fd));
                    FindClose(hFind);
                }
                RemoveEmptyDirectory(nvdDir);
            }
        }

        // 3. Delete files in Control Panel Client directory
        CleanDirectoryFiles(dstDir);
        RemoveEmptyDirectory(dstDir);

        // Remove empty parent NVIDIA Corporation if empty
        wchar_t parentDir[MAX_PATH];
        wcscpy_s(parentDir, MAX_PATH, dstDir);
        wchar_t* lastSlash = wcsrchr(parentDir, L'\\');
        if (lastSlash && lastSlash > parentDir) {
            *lastSlash = L'\0';
            lastSlash = wcsrchr(parentDir, L'\\');
            if (lastSlash) {
                *(lastSlash + 1) = L'\0';
                RemoveEmptyDirectory(parentDir);
            }
        }

        // 4. Delete registry entries
        DeleteRegKeyRecursive(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\nvcplui.exe");
        DeleteRegKeyRecursive(HKEY_LOCAL_MACHINE, L"SOFTWARE\\Classes\\DesktopBackground\\Shell\\NvControlPanel");

        HKEY hTweak;
        if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SYSTEM\\CurrentControlSet\\Services\\nvlddmkm\\Parameters\\Global\\NVTweak", 0, KEY_SET_VALUE, &hTweak) == ERROR_SUCCESS) {
            RegDeleteValueW(hTweak, L"EnableNvcplUIOnDCH");
            RegCloseKey(hTweak);
        }
        if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\NVIDIA Corporation\\Global\\NVTweak", 0, KEY_SET_VALUE, &hTweak) == ERROR_SUCCESS) {
            RegDeleteValueW(hTweak, L"EnableNvcplUIOnDCH");
            RegCloseKey(hTweak);
        }

        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_FLUSH, NULL, NULL);
        LogMsg(L"install_helper: UNINSTALL complete.");
        return 0;
    }

    // =========================================================================
    // INSTALL MODE
    // =========================================================================
    LogMsg(L"install_helper: INSTALL mode started.");

    // 1. Get current exe directory
    wchar_t srcDir[MAX_PATH];
    GetModuleFileNameW(NULL, srcDir, MAX_PATH);
    wchar_t* lastSlash = wcsrchr(srcDir, L'\\');
    if (lastSlash) {
        *(lastSlash + 1) = L'\0';
    }

    swprintf_s(logBuf, 1024, L"Source: %s, Destination: %s", srcDir, dstDir);
    LogMsg(logBuf);

    // 2. Create target directory
    wchar_t partDir[MAX_PATH];
    wcscpy_s(partDir, MAX_PATH, dstDir);
    wchar_t* p = partDir;
    while (*p) {
        if (*p == L'\\') {
            *p = L'\0';
            CreateDirectoryW(partDir, NULL);
            *p = L'\\';
        }
        p++;
    }
    CreateDirectoryW(dstDir, NULL);

    // 3. Files to copy
    const wchar_t* files[] = {
        L"nvcplui.exe",
        L"nvcpluir.dll",
        L"NvGpuUtilization.exe",
        L"nvImage.dll",
        L"EULA.txt",
        L"NVCPL.nvi",
        L"install_helper.exe"
    };

    int copied = 0;
    for (int i = 0; i < sizeof(files)/sizeof(files[0]); i++) {
        wchar_t srcFile[MAX_PATH];
        wchar_t dstFile[MAX_PATH];
        swprintf_s(srcFile, MAX_PATH, L"%s%s", srcDir, files[i]);
        swprintf_s(dstFile, MAX_PATH, L"%s%s", dstDir, files[i]);

        if (CopyFileW(srcFile, dstFile, FALSE)) {
            swprintf_s(logBuf, 1024, L"Copied: %s -> %s", files[i], dstFile);
            LogMsg(logBuf);
            copied++;
        } else {
            DWORD err = GetLastError();
            swprintf_s(logBuf, 1024, L"FAILED copy %s (error %u)", files[i], err);
            LogMsg(logBuf);
        }
    }

    // 4. Create Shortcuts natively with clean Shell Link format
    wchar_t exePath[MAX_PATH];
    swprintf_s(exePath, MAX_PATH, L"%snvcplui.exe", dstDir);

    // Desktop Shortcut: NVIDIA Control Panel.lnk
    wchar_t desktopPath[MAX_PATH];
    if (SUCCEEDED(SHGetFolderPathW(NULL, CSIDL_COMMON_DESKTOPDIRECTORY, NULL, 0, desktopPath))) {
        // Clean old variants
        wchar_t searchOldD[MAX_PATH];
        swprintf_s(searchOldD, MAX_PATH, L"%s\\*.lnk", desktopPath);
        WIN32_FIND_DATAW fd;
        HANDLE hFind = FindFirstFileW(searchOldD, &fd);
        if (hFind != INVALID_HANDLE_VALUE) {
            do {
                if (wcsstr(fd.cFileName, L"Панель") || wcsstr(fd.cFileName, L"Рџ")) {
                    wchar_t oldLnk[MAX_PATH];
                    swprintf_s(oldLnk, MAX_PATH, L"%s\\%s", desktopPath, fd.cFileName);
                    DeleteFileW(oldLnk);
                    SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, oldLnk, NULL);
                }
            } while (FindNextFileW(hFind, &fd));
            FindClose(hFind);
        }

        wchar_t lnkDesktop[MAX_PATH];
        swprintf_s(lnkDesktop, MAX_PATH, L"%s\\NVIDIA Control Panel.lnk", desktopPath);
        HRESULT hr = CreateCleanShortcut(lnkDesktop, exePath, L"", dstDir, exePath, 0, L"NVIDIA Control Panel");
        swprintf_s(logBuf, 1024, L"Created Desktop shortcut: %s (hr=0x%08X)", lnkDesktop, hr);
        LogMsg(logBuf);
        SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, lnkDesktop, NULL);
    }

    // Start Menu Shortcut: NVIDIA Control Panel.lnk
    wchar_t startPath[MAX_PATH];
    if (SUCCEEDED(SHGetFolderPathW(NULL, CSIDL_COMMON_PROGRAMS, NULL, 0, startPath))) {
        wchar_t nvdStartDir[MAX_PATH];
        swprintf_s(nvdStartDir, MAX_PATH, L"%s\\NVIDIA Corporation", startPath);
        CreateDirectoryW(nvdStartDir, NULL);

        // Clean any old krakozyabry or duplicate shortcuts in NVIDIA Corporation folder
        wchar_t searchOld[MAX_PATH];
        swprintf_s(searchOld, MAX_PATH, L"%s\\*.lnk", nvdStartDir);
        WIN32_FIND_DATAW fd;
        HANDLE hFind = FindFirstFileW(searchOld, &fd);
        if (hFind != INVALID_HANDLE_VALUE) {
            do {
                if (wcsstr(fd.cFileName, L"Панель") || wcsstr(fd.cFileName, L"Удалить") || 
                    wcsstr(fd.cFileName, L"Рџ") || wcsstr(fd.cFileName, L"РЈ")) {
                    wchar_t oldLnk[MAX_PATH];
                    swprintf_s(oldLnk, MAX_PATH, L"%s\\%s", nvdStartDir, fd.cFileName);
                    DeleteFileW(oldLnk);
                    SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, oldLnk, NULL);
                }
            } while (FindNextFileW(hFind, &fd));
            FindClose(hFind);
        }

        // App shortcut
        wchar_t lnkStart[MAX_PATH];
        swprintf_s(lnkStart, MAX_PATH, L"%s\\NVIDIA Control Panel.lnk", nvdStartDir);
        HRESULT hr = CreateCleanShortcut(lnkStart, exePath, L"", dstDir, exePath, 0, L"NVIDIA Control Panel");
        swprintf_s(logBuf, 1024, L"Created Start shortcut: %s (hr=0x%08X)", lnkStart, hr);
        LogMsg(logBuf);
        SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, lnkStart, NULL);
    }

    // Registry: NVTweak EnableNvcplUIOnDCH = 1
    HKEY hKey;
    if (RegCreateKeyExW(HKEY_LOCAL_MACHINE, L"SYSTEM\\CurrentControlSet\\Services\\nvlddmkm\\Parameters\\Global\\NVTweak", 0, NULL, 0, KEY_SET_VALUE, NULL, &hKey, NULL) == ERROR_SUCCESS) {
        DWORD val = 1;
        RegSetValueExW(hKey, L"EnableNvcplUIOnDCH", 0, REG_DWORD, (BYTE*)&val, sizeof(DWORD));
        RegCloseKey(hKey);
    }
    if (RegCreateKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\NVIDIA Corporation\\Global\\NVTweak", 0, NULL, 0, KEY_SET_VALUE, NULL, &hKey, NULL) == ERROR_SUCCESS) {
        DWORD val = 1;
        RegSetValueExW(hKey, L"EnableNvcplUIOnDCH", 0, REG_DWORD, (BYTE*)&val, sizeof(DWORD));
        RegCloseKey(hKey);
    }

    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_FLUSH, NULL, NULL);

    swprintf_s(logBuf, 1024, L"install_helper finished. Copied %d files.", copied);
    LogMsg(logBuf);
    return 0;
}
