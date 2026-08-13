//! Windows lid-close sleep suppression.
//!
//! The journal is authoritative. It is written before any power setting is
//! changed and is removed only after every touched scheme has been restored.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::Emitter;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, GENERIC_READ,
    GENERIC_WRITE, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WAIT_ABANDONED,
    WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, CREATE_NEW,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, OPEN_EXISTING,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Power::{
    GetPwrCapabilities, PowerEnumerate, PowerGetActiveScheme, PowerReadACValueIndex,
    PowerReadDCValueIndex, PowerSettingAccessCheck, PowerWriteACValueIndex, PowerWriteDCValueIndex,
    RegisterPowerSettingNotification, SetThreadExecutionState, UnregisterPowerSettingNotification,
    ACCESS_AC_POWER_SETTING_INDEX, ACCESS_DC_POWER_SETTING_INDEX, ACCESS_SCHEME, ES_CONTINUOUS,
    ES_SYSTEM_REQUIRED, POWERBROADCAST_SETTING, SYSTEM_POWER_CAPABILITIES,
};
use windows_sys::Win32::System::SystemServices::{
    GUID_ACTIVE_POWERSCHEME, GUID_LIDCLOSE_ACTION, GUID_LIDSWITCH_STATE_CHANGE,
    GUID_SYSTEM_BUTTON_SUBGROUP,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, CreateProcessW, DeleteProcThreadAttributeList,
    InitializeProcThreadAttributeList, OpenMutexW, SetEvent, TerminateProcess,
    UpdateProcThreadAttribute, WaitForMultipleObjects, WaitForSingleObject,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, STARTUPINFOEXW, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, SetTimer, DEVICE_NOTIFY_WINDOW_HANDLE, MSG, PBT_POWERSETTINGCHANGE, WM_DESTROY,
    WM_ENDSESSION, WM_POWERBROADCAST, WM_QUERYENDSESSION, WM_TIMER, WNDCLASSW,
};

const HELPER_ARG: &str = "--switcher-clamshell-windows-helper";
const STATE_FILE: &str = "clamshell-windows-state.json";
const RECOVERY_FILE: &str = "clamshell-windows-recovery.json";
const STOP_FILE: &str = "clamshell-windows-stop";
const HELPER_FILE_PREFIX: &str = "clamshell-windows-helper-";
const TRANSITION_MUTEX: &str = "Global\\SwitcherClamshellWindowsTransition-v1";
const OWNER_MUTEX: &str = "Global\\SwitcherClamshellWindowsOwner-v1";
const TIMER_ID: usize = 1;

static OPERATION_LOCK: Mutex<()> = Mutex::new(());
static HELPER: OnceLock<Mutex<HelperRuntime>> = OnceLock::new();
static WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SchemeJournal {
    scheme: String,
    ac: u32,
    dc: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    version: u8,
    mode: u8,
    revision: String,
    helper: String,
    schemes: Vec<SchemeJournal>,
}

trait PowerBackend {
    fn has_lid(&mut self) -> Result<bool, String>;
    fn check_access(&mut self) -> Result<(), String>;
    fn active_scheme(&mut self) -> Result<String, String>;
    fn scheme_exists(&mut self, scheme: &str) -> Result<bool, String>;
    fn read_actions(&mut self, scheme: &str) -> Result<(u32, u32), String>;
    fn write_ac(&mut self, scheme: &str, value: u32) -> Result<(), String>;
    fn write_dc(&mut self, scheme: &str, value: u32) -> Result<(), String>;
    fn apply_current_lid_settings(&mut self) -> Result<(), String>;
    fn request_system(&mut self) -> Result<(), String>;
    fn clear_request(&mut self);
}

struct NativePower;

impl PowerBackend for NativePower {
    fn has_lid(&mut self) -> Result<bool, String> {
        let mut caps: SYSTEM_POWER_CAPABILITIES = unsafe { std::mem::zeroed() };
        if unsafe { GetPwrCapabilities(&mut caps) } {
            Ok(caps.LidPresent)
        } else {
            Err(last_error("노트북 덮개 확인 실패"))
        }
    }

    fn check_access(&mut self) -> Result<(), String> {
        for accessor in [ACCESS_AC_POWER_SETTING_INDEX, ACCESS_DC_POWER_SETTING_INDEX] {
            let status = unsafe { PowerSettingAccessCheck(accessor, &GUID_LIDCLOSE_ACTION) };
            win32(status, "덮개 동작 변경 권한이 없거나 그룹 정책으로 차단됨")?;
        }
        // This API is resolved dynamically because it is not in every Windows SDK.
        // Prove that the running OS can apply the lid setting before journaling or
        // changing any AC/DC value; otherwise activation could leave a journal that
        // can never be cleared by the same unavailable operation.
        self.apply_current_lid_settings()
    }

    fn active_scheme(&mut self) -> Result<String, String> {
        let mut ptr = std::ptr::null_mut();
        win32(
            unsafe { PowerGetActiveScheme(std::ptr::null_mut(), &mut ptr) },
            "활성 전원 관리 옵션 확인 실패",
        )?;
        if ptr.is_null() {
            return Err("활성 전원 관리 옵션 확인 실패: 빈 GUID".into());
        }
        let guid = unsafe { *ptr };
        unsafe { LocalFree(ptr.cast()) };
        Ok(guid_text(&guid))
    }

    fn scheme_exists(&mut self, scheme: &str) -> Result<bool, String> {
        let wanted = parse_guid(scheme)?;
        let mut index = 0;
        loop {
            let mut candidate: GUID = unsafe { std::mem::zeroed() };
            let mut size = std::mem::size_of::<GUID>() as u32;
            let status = unsafe {
                PowerEnumerate(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    ACCESS_SCHEME,
                    index,
                    (&mut candidate as *mut GUID).cast(),
                    &mut size,
                )
            };
            match status {
                ERROR_SUCCESS => {
                    if size as usize == std::mem::size_of::<GUID>() && guid_eq(&candidate, &wanted)
                    {
                        return Ok(true);
                    }
                    index += 1;
                }
                ERROR_NO_MORE_ITEMS => return Ok(false),
                error => {
                    return Err(format!(
                        "전원 관리 옵션 목록 확인 실패 (Windows 오류 {error})"
                    ));
                }
            }
        }
    }

    fn read_actions(&mut self, scheme: &str) -> Result<(u32, u32), String> {
        let guid = parse_guid(scheme)?;
        let mut ac = 0;
        let mut dc = 0;
        win32(
            unsafe {
                PowerReadACValueIndex(
                    std::ptr::null_mut(),
                    &guid,
                    &GUID_SYSTEM_BUTTON_SUBGROUP,
                    &GUID_LIDCLOSE_ACTION,
                    &mut ac,
                )
            },
            "AC 덮개 동작 읽기 실패",
        )?;
        win32(
            unsafe {
                PowerReadDCValueIndex(
                    std::ptr::null_mut(),
                    &guid,
                    &GUID_SYSTEM_BUTTON_SUBGROUP,
                    &GUID_LIDCLOSE_ACTION,
                    &mut dc,
                )
            },
            "DC 덮개 동작 읽기 실패",
        )?;
        Ok((ac, dc))
    }

    fn write_ac(&mut self, scheme: &str, value: u32) -> Result<(), String> {
        let guid = parse_guid(scheme)?;
        win32(
            unsafe {
                PowerWriteACValueIndex(
                    std::ptr::null_mut(),
                    &guid,
                    &GUID_SYSTEM_BUTTON_SUBGROUP,
                    &GUID_LIDCLOSE_ACTION,
                    value,
                )
            },
            "AC 덮개 동작 쓰기 실패",
        )
    }

    fn write_dc(&mut self, scheme: &str, value: u32) -> Result<(), String> {
        let guid = parse_guid(scheme)?;
        win32(
            unsafe {
                PowerWriteDCValueIndex(
                    std::ptr::null_mut(),
                    &guid,
                    &GUID_SYSTEM_BUTTON_SUBGROUP,
                    &GUID_LIDCLOSE_ACTION,
                    value,
                )
            },
            "DC 덮개 동작 쓰기 실패",
        )
    }

    fn apply_current_lid_settings(&mut self) -> Result<(), String> {
        type ApplySettingChanges = unsafe extern "system" fn(*const GUID, *const GUID) -> u32;
        static APPLY: OnceLock<Result<ApplySettingChanges, String>> = OnceLock::new();
        let apply =
            APPLY
                .get_or_init(|| {
                    let module = unsafe { LoadLibraryW(wide("powrprof.dll").as_ptr()) };
                    if module.is_null() {
                        return Err(last_error("전원 설정 적용 함수 불러오기 실패"));
                    }
                    let Some(address) =
                        (unsafe { GetProcAddress(module, b"PowerApplySettingChanges\0".as_ptr()) })
                    else {
                        return Err(last_error(
                            "이 Windows 버전은 안전한 전원 설정 적용을 지원하지 않음",
                        ));
                    };
                    Ok(unsafe {
                        std::mem::transmute::<
                            unsafe extern "system" fn() -> isize,
                            ApplySettingChanges,
                        >(address)
                    })
                })
                .as_ref()
                .map_err(Clone::clone)?;
        win32(
            unsafe { apply(&GUID_SYSTEM_BUTTON_SUBGROUP, &GUID_LIDCLOSE_ACTION) },
            "덮개 전원 설정 적용 실패",
        )
    }

    fn request_system(&mut self) -> Result<(), String> {
        let result = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        if result == 0 {
            Err(last_error("절전 방지 요청 실패"))
        } else {
            Ok(())
        }
    }

    fn clear_request(&mut self) {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

struct RequestGuard {
    backend: NativePower,
    held: bool,
}

impl RequestGuard {
    fn acquire(state_path: &Path) -> Result<Self, String> {
        let mut backend = NativePower;
        acquire_request_after_journal(&mut backend, state_path)?;
        Ok(Self {
            backend,
            held: true,
        })
    }

    fn clear(&mut self) {
        if self.held {
            self.backend.clear_request();
            self.held = false;
        }
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.clear();
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

struct NamedOperation(OwnedHandle);

impl NamedOperation {
    fn lock(helper: &str) -> Result<Self, String> {
        if !valid_token(helper) {
            return Err("클램셸 작업 식별자가 올바르지 않습니다".into());
        }
        let name = wide(&operation_mutex_name(helper));
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_error("클램셸 작업 잠금 생성 실패"));
        }
        let owned = OwnedHandle(handle);
        if !matches!(
            unsafe { WaitForSingleObject(handle, 10_000) },
            WAIT_OBJECT_0 | WAIT_ABANDONED
        ) {
            return Err("다른 클램셸 작업이 끝나지 않았습니다".into());
        }
        Ok(Self(owned))
    }
}

struct NamedTransition(OwnedHandle);

impl NamedTransition {
    fn lock() -> Result<Self, String> {
        let name = wide(TRANSITION_MUTEX);
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_error("클램셸 인수인계 잠금 생성 실패"));
        }
        let owned = OwnedHandle(handle);
        if !matches!(
            unsafe { WaitForSingleObject(handle, 2_000) },
            WAIT_OBJECT_0 | WAIT_ABANDONED
        ) {
            return Err("다른 Windows 세션의 클램셸 인수인계가 끝나지 않았습니다".into());
        }
        Ok(Self(owned))
    }
}

struct NamedOwner(OwnedHandle);

impl NamedOwner {
    fn acquire() -> Result<Self, String> {
        let name = wide(OWNER_MUTEX);
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_error("다른 Windows 세션의 클램셸 소유권 확인 실패"));
        }
        let owned = OwnedHandle(handle);
        if !matches!(
            unsafe { WaitForSingleObject(handle, 0) },
            WAIT_OBJECT_0 | WAIT_ABANDONED
        ) {
            return Err("다른 Windows 로그인 세션에서 클램셸 모드를 사용 중입니다".into());
        }
        Ok(Self(owned))
    }
}

impl Drop for NamedOwner {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0 .0);
        }
    }
}

impl Drop for NamedOperation {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0 .0);
        }
    }
}

impl Drop for NamedTransition {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0 .0);
        }
    }
}

fn files(store: &Path) -> PathBuf {
    store.join(STATE_FILE)
}

fn recovery_file(path: &Path) -> Result<PathBuf, String> {
    Ok(path
        .parent()
        .ok_or("클램셸 상태 폴더가 없습니다")?
        .join(RECOVERY_FILE))
}

fn stop_file(path: &Path) -> Result<PathBuf, String> {
    Ok(path
        .parent()
        .ok_or("클램셸 상태 폴더가 없습니다")?
        .join(STOP_FILE))
}

fn token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(format!("클램셸 식별자 생성 실패 (NTSTATUS 0x{status:08x})"));
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn parse_state(bytes: &[u8]) -> Result<State, String> {
    let state: State = serde_json::from_slice(&bytes)
        .map_err(|error| format!("클램셸 상태 형식 오류: {error}"))?;
    if state.version != 1
        || !(1..=3).contains(&state.mode)
        || !valid_token(&state.revision)
        || !valid_token(&state.helper)
        || state.schemes.is_empty()
        || state.schemes.iter().any(|entry| {
            parse_guid(&entry.scheme).is_err()
                || entry.ac > 3
                || entry.dc > 3
                || state
                    .schemes
                    .iter()
                    .filter(|other| other.scheme == entry.scheme)
                    .count()
                    != 1
        })
    {
        return Err("클램셸 상태 값이 올바르지 않습니다".into());
    }
    Ok(state)
}

fn read_state_file(path: &Path) -> Result<Option<State>, String> {
    match std::fs::read(path) {
        Ok(bytes) => parse_state(&bytes).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("클램셸 상태 읽기 실패: {error}")),
    }
}

fn read_state(path: &Path) -> Result<Option<State>, String> {
    match read_state_file(path) {
        Ok(Some(state)) => Ok(Some(state)),
        primary => {
            let backup = read_state_file(&recovery_file(path)?)?;
            match (primary, backup) {
                (_, Some(state)) => Ok(Some(state)),
                (Ok(None), None) => Ok(None),
                (Err(error), None) => Err(error),
                (Ok(Some(_)), _) => unreachable!(),
            }
        }
    }
}

fn write_state(path: &Path, state: &State) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(state).map_err(|error| format!("클램셸 상태 직렬화 실패: {error}"))?;
    let stop = stop_file(path)?;
    match std::fs::remove_file(&stop) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("이전 클램셸 종료 표식 정리 실패: {error}")),
    }
    crate::accounts::atomic_write_existing_parent(path, &bytes)
        .map_err(|error| format!("클램셸 상태 저장 실패: {error}"))?;
    crate::accounts::atomic_write_existing_parent(&recovery_file(path)?, &bytes)
        .map_err(|error| format!("클램셸 복구 상태 저장 실패: {error}"))
}

fn remove_file_if_present(path: &Path, context: &str) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{context}: {error}")),
    }
}

fn remove_state(path: &Path, helper: &str) -> Result<(), String> {
    crate::accounts::atomic_write_existing_parent(&stop_file(path)?, helper.as_bytes())
        .map_err(|error| format!("클램셸 종료 표식 저장 실패: {error}"))?;
    remove_file_if_present(&recovery_file(path)?, "클램셸 복구 상태 정리 실패")?;
    remove_file_if_present(path, "클램셸 상태 정리 실패")
}

fn intentionally_stopped(path: &Path, helper: &str) -> bool {
    let Ok(stop) = stop_file(path) else {
        return false;
    };
    std::fs::read_to_string(stop)
        .ok()
        .is_some_and(|value| value == helper)
}

fn apply_active<B: PowerBackend>(
    backend: &mut B,
    state_path: &Path,
    state: &mut State,
) -> Result<(), String> {
    let scheme = backend.active_scheme()?;
    let (ac, dc) = backend.read_actions(&scheme)?;
    let mut journal_changed = false;
    if let Some(entry) = state
        .schemes
        .iter_mut()
        .find(|entry| entry.scheme == scheme)
    {
        // A zero baseline means this helper did not cause that field to become
        // zero. If it later sees a nonzero value, it must claim and journal that
        // value before suppressing it so chained helper crashes remain lossless.
        if entry.ac == 0 && ac != 0 {
            entry.ac = ac;
            journal_changed = true;
        }
        if entry.dc == 0 && dc != 0 {
            entry.dc = dc;
            journal_changed = true;
        }
    } else {
        state.schemes.push(SchemeJournal {
            scheme: scheme.clone(),
            ac,
            dc,
        });
        journal_changed = true;
    }
    if journal_changed {
        write_state(state_path, state)?;
    }
    ensure_ignored(backend, &scheme, ac, dc)
}

fn activate<B: PowerBackend>(
    backend: &mut B,
    state_path: &Path,
    state: &mut State,
) -> Result<(), String> {
    backend.check_access()?;
    apply_active(backend, state_path, state)
}

fn ensure_ignored<B: PowerBackend>(
    backend: &mut B,
    scheme: &str,
    old_ac: u32,
    old_dc: u32,
) -> Result<(), String> {
    if old_ac == 0 && old_dc == 0 {
        return Ok(());
    }
    backend.write_ac(scheme, 0)?;
    if let Err(error) = backend.write_dc(scheme, 0) {
        let rollback = backend.write_ac(scheme, old_ac);
        return Err(combine_rollback(error, rollback));
    }
    if let Err(error) = backend.apply_current_lid_settings() {
        let ac = backend.write_ac(scheme, old_ac);
        let dc = backend.write_dc(scheme, old_dc);
        return Err(combine_rollbacks(error, ac, dc));
    }
    Ok(())
}

fn restore_all<B: PowerBackend>(
    backend: &mut B,
    state_path: &Path,
    state: &State,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for entry in &state.schemes {
        match backend.scheme_exists(&entry.scheme) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                errors.push(error);
                continue;
            }
        }
        // Zero is both the target and the ownership sentinel: this helper did
        // not change an already-zero field, so it must never "restore" it over
        // the real owner's later recovery in another login session.
        if entry.ac != 0 {
            if let Err(error) = backend.write_ac(&entry.scheme, entry.ac) {
                errors.push(error);
            }
        }
        if entry.dc != 0 {
            if let Err(error) = backend.write_dc(&entry.scheme, entry.dc) {
                errors.push(error);
            }
        }
    }
    if let Err(error) = backend.apply_current_lid_settings() {
        errors.push(error);
    }
    backend.clear_request();
    if errors.is_empty() {
        remove_state(state_path, &state.helper)
    } else {
        Err(format!("전원 설정 복원 실패: {}", errors.join("; ")))
    }
}

fn recover_dead<B: PowerBackend>(
    backend: &mut B,
    state_path: &Path,
    state: &State,
    helper_alive: bool,
) -> Result<bool, String> {
    if helper_alive {
        Ok(false)
    } else {
        restore_all(backend, state_path, state)?;
        Ok(true)
    }
}

fn acquire_request_after_journal<B: PowerBackend>(
    backend: &mut B,
    state_path: &Path,
) -> Result<(), String> {
    if read_state(state_path)?.is_none() {
        return Err("클램셸 상태를 기록하기 전에 절전 방지를 시작할 수 없습니다".into());
    }
    backend.request_system()
}

pub fn mode(store: &Path) -> i8 {
    match read_state(&files(store)) {
        Ok(Some(state)) => {
            if intentionally_stopped(&files(store), &state.helper) {
                return 0;
            }
            return state.mode.min(2) as i8;
        }
        Err(error) => {
            eprintln!("클램셸 상태 확인 실패: {error}");
            return 2;
        }
        Ok(None) => {}
    }
    let mut backend = NativePower;
    if !matches!(backend.has_lid(), Ok(true)) {
        return -1;
    }
    0
}

pub fn cycle(app: &tauri::AppHandle, store: &Path) -> Result<i8, String> {
    let _local = OPERATION_LOCK
        .lock()
        .map_err(|_| "클램셸 작업 잠금 손상".to_string())?;
    let path = files(store);
    let mut backend = NativePower;
    let candidate = read_state(&path)?;
    let operation_helper = candidate
        .as_ref()
        .map(|state| Ok(state.helper.clone()))
        .unwrap_or_else(token)?;
    let _named = NamedOperation::lock(&operation_helper)?;
    let existing = read_state(&path)?;
    if existing.as_ref().map(|state| &state.helper) != candidate.as_ref().map(|state| &state.helper)
    {
        return Err("클램셸 상태가 전환 중 바뀌었습니다. 다시 눌러 주세요".into());
    }
    if existing.is_none() && !backend.has_lid()? {
        return Err("이 컴퓨터에는 노트북 덮개가 없습니다".into());
    }
    if let Some(state) = existing.as_ref() {
        if !helper_alive(&state.helper) {
            let _transition = NamedTransition::lock()?;
            let _owner = NamedOwner::acquire()?;
            let Some(locked_state) = read_state(&path)? else {
                return Ok(0);
            };
            if locked_state.helper != state.helper {
                return Err("클램셸 감시자가 인수인계 중 바뀌었습니다. 다시 눌러 주세요".into());
            }
            if helper_alive(&locked_state.helper) {
                return Ok(locked_state.mode as i8);
            }
            recover_dead(&mut backend, &path, &locked_state, false)?;
            let _ = app.emit("clamshell-changed", 0i8);
            return Ok(0);
        }
    }
    match existing {
        None => {
            let _transition = NamedTransition::lock()?;
            let owner = NamedOwner::acquire()?;
            if read_state(&path)?.is_some() {
                return Err("클램셸 상태가 인수인계 중 바뀌었습니다. 다시 눌러 주세요".into());
            }
            let helper = operation_helper;
            cleanup_stale_helpers(store, None);
            let mut state = State {
                version: 1,
                mode: 1,
                revision: token()?,
                helper: helper.clone(),
                schemes: Vec::new(),
            };
            if let Err(error) = activate(&mut backend, &path, &mut state) {
                let rollback = restore_all(&mut backend, &path, &state);
                return Err(combine_rollback(error, rollback));
            }
            drop(owner);
            if let Err(error) = spawn_helper(&path, &helper) {
                let rollback = restore_all(&mut backend, &path, &state);
                return Err(combine_rollback(error, rollback));
            }
            let _ = app.emit("clamshell-changed", 1i8);
            Ok(1)
        }
        Some(mut state) if state.mode == 1 => {
            state.mode = 2;
            state.revision = token()?;
            write_state(&path, &state)?;
            let _ = app.emit("clamshell-changed", 2i8);
            Ok(2)
        }
        Some(mut state) => {
            // Persist the restoring phase before touching power settings. If
            // the GUI dies here, the live helper will finish the restore and
            // will never interpret the remaining journal as an active mode.
            if state.mode != 3 {
                state.mode = 3;
                state.revision = token()?;
                write_state(&path, &state)?;
            }
            restore_all(&mut backend, &path, &state)?;
            let _ = app.emit("clamshell-changed", 0i8);
            Ok(0)
        }
    }
}

pub fn on_quit(_store: &Path) {
    // The helper deliberately survives an ordinary app quit/restart. Windows
    // session shutdown is handled by its native WM_ENDSESSION hook.
}

fn reconcile_dead_helper(app: &tauri::AppHandle, store: &Path) {
    let path = files(store);
    let candidate = match read_state(&path) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("클램셸 시작 상태 확인 실패: {error}");
            return;
        }
    };
    let Some(candidate) = candidate else {
        return;
    };
    if helper_alive(&candidate.helper) {
        cleanup_stale_helpers(store, Some(&candidate.helper));
        return;
    }
    let Ok(_local) = OPERATION_LOCK.lock() else {
        return;
    };
    let Ok(_named) = NamedOperation::lock(&candidate.helper) else {
        return;
    };
    let Ok(_transition) = NamedTransition::lock() else {
        return;
    };
    let Ok(_owner) = NamedOwner::acquire() else {
        return;
    };
    let state = match read_state(&path) {
        Ok(Some(state)) => state,
        Ok(None) => return,
        Err(error) => {
            eprintln!("클램셸 잠금 후 상태 재확인 실패: {error}");
            return;
        }
    };
    if state.helper != candidate.helper {
        return;
    }
    if intentionally_stopped(&path, &state.helper) {
        if let Err(error) = remove_state(&path, &state.helper) {
            eprintln!("클램셸 종료 상태 정리 재시도 실패: {error}");
        } else {
            cleanup_stale_helpers(store, None);
            let _ = app.emit("clamshell-changed", 0i8);
        }
        return;
    }
    if helper_alive(&state.helper) {
        cleanup_stale_helpers(store, Some(&state.helper));
        return;
    }
    let mut backend = NativePower;
    if let Err(error) = recover_dead(&mut backend, &path, &state, false) {
        eprintln!("클램셸 감시자 복구 실패: {error}");
    } else {
        cleanup_stale_helpers(store, None);
        let _ = app.emit("clamshell-changed", 0i8);
    }
}

pub fn on_start(app: &tauri::AppHandle, store: &Path) {
    reconcile_dead_helper(app, store);
    if WATCHDOG_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let store = store.to_path_buf();
    std::thread::spawn(move || {
        let mut last_mode = mode(&store);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            reconcile_dead_helper(&app, &store);
            let current_mode = mode(&store);
            if current_mode != last_mode {
                last_mode = current_mode;
                let _ = app.emit("clamshell-changed", current_mode);
            }
        }
    });
}

pub fn run_helper_if_requested() -> bool {
    let mut args = std::env::args_os();
    let _exe = args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new(HELPER_ARG)) {
        return false;
    }
    let Some(path) = args.next().map(PathBuf::from) else {
        return true;
    };
    let Some(helper) = args.next().and_then(|value| value.into_string().ok()) else {
        return true;
    };
    if valid_token(&helper) {
        if let Err(error) = helper_main(path, helper) {
            eprintln!("클램셸 감시자 실패: {error}");
        }
    }
    true
}

struct ProcessAttributes {
    storage: Box<[usize]>,
    policy: Box<u64>,
    pointer: windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl ProcessAttributes {
    fn system32_first() -> Result<Self, String> {
        let mut bytes = 0usize;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(last_error("감시자 보안 속성 크기 확인 실패"));
        }
        let words = bytes.div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0usize; words].into_boxed_slice();
        let pointer = storage.as_mut_ptr().cast();
        if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut bytes) } == 0 {
            return Err(last_error("감시자 보안 속성 초기화 실패"));
        }
        let policy = Box::new(1u64 << 60);
        if unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY as usize,
                (&*policy as *const u64).cast(),
                std::mem::size_of::<u64>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            unsafe { DeleteProcThreadAttributeList(pointer) };
            return Err(last_error("감시자 DLL 검색 보호 설정 실패"));
        }
        Ok(Self {
            storage,
            policy,
            pointer,
        })
    }
}

impl Drop for ProcessAttributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.pointer) };
        let _ = (&self.storage, &self.policy);
    }
}

fn quote_windows_arg(value: &std::ffi::OsStr, output: &mut Vec<u16>) {
    use std::os::windows::ffi::OsStrExt;

    output.push(b'"' as u16);
    let mut slashes = 0usize;
    for unit in value.encode_wide() {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else if unit == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            output.push(unit);
            slashes = 0;
        } else {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            output.push(unit);
            slashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
}

fn helper_command_line(exe: &Path, state_path: &Path, helper: &str) -> Vec<u16> {
    let mut output = Vec::new();
    for (index, value) in [
        exe.as_os_str(),
        std::ffi::OsStr::new(HELPER_ARG),
        state_path.as_os_str(),
        std::ffi::OsStr::new(helper),
    ]
    .into_iter()
    .enumerate()
    {
        if index != 0 {
            output.push(b' ' as u16);
        }
        quote_windows_arg(value, &mut output);
    }
    output.push(0);
    output
}

fn create_helper_copy(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;

    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            destination_wide.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error("클램셸 감시자 파일 생성 실패"));
    }
    let mut destination_file = unsafe { std::fs::File::from_raw_handle(handle.cast()) };
    let result = (|| {
        let mut source_file =
            std::fs::File::open(source).map_err(|error| format!("실행 파일 열기 실패: {error}"))?;
        std::io::copy(&mut source_file, &mut destination_file)
            .map_err(|error| format!("클램셸 감시자 복사 실패: {error}"))?;
        destination_file
            .sync_all()
            .map_err(|error| format!("클램셸 감시자 동기화 실패: {error}"))
    })();
    drop(destination_file);
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

fn open_verified_helper_copy(source: &Path, destination: &Path) -> Result<std::fs::File, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            destination_wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error("클램셸 감시자 검증용 열기 실패"));
    }
    let mut destination_file = unsafe { std::fs::File::from_raw_handle(handle.cast()) };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe {
        GetFileInformationByHandle(
            destination_file.as_raw_handle().cast(),
            &mut info as *mut BY_HANDLE_FILE_INFORMATION,
        )
    } == 0
    {
        return Err(last_error("클램셸 감시자 파일 정보 확인 실패"));
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("클램셸 감시자 복사본이 재분석 지점으로 바뀌었습니다".into());
    }

    let mut source_file =
        std::fs::File::open(source).map_err(|error| format!("실행 파일 재검증 실패: {error}"))?;
    let mut source_buffer = [0u8; 64 * 1024];
    let mut destination_buffer = [0u8; 64 * 1024];
    loop {
        let source_read = std::io::Read::read(&mut source_file, &mut source_buffer)
            .map_err(|error| format!("실행 파일 검증 읽기 실패: {error}"))?;
        let destination_read = std::io::Read::read(&mut destination_file, &mut destination_buffer)
            .map_err(|error| format!("클램셸 감시자 검증 읽기 실패: {error}"))?;
        if source_read != destination_read
            || source_buffer[..source_read] != destination_buffer[..destination_read]
        {
            return Err("클램셸 감시자 복사본 검증 실패".into());
        }
        if source_read == 0 {
            break;
        }
    }
    Ok(destination_file)
}

fn spawn_helper(state_path: &Path, helper: &str) -> Result<(), String> {
    let ready_name = ready_name(helper);
    let ready_wide = wide(&ready_name);
    let ready = unsafe { CreateEventW(std::ptr::null(), 1, 0, ready_wide.as_ptr()) };
    if ready.is_null() {
        return Err(last_error("클램셸 감시자 준비 이벤트 생성 실패"));
    }
    let ready = OwnedHandle(ready);
    let exe =
        std::env::current_exe().map_err(|error| format!("실행 파일 경로 확인 실패: {error}"))?;
    let helper_exe = helper_copy_path(state_path, helper)?;
    if helper_exe == exe {
        return Err("클램셸 감시자는 본 실행 파일과 분리되어야 합니다".into());
    }
    create_helper_copy(&exe, &helper_exe)?;
    let helper_file = match open_verified_helper_copy(&exe, &helper_exe) {
        Ok(file) => file,
        Err(error) => {
            let _ = std::fs::remove_file(&helper_exe);
            return Err(error);
        }
    };
    let attributes = match ProcessAttributes::system32_first() {
        Ok(attributes) => attributes,
        Err(error) => {
            drop(helper_file);
            let _ = std::fs::remove_file(&helper_exe);
            return Err(error);
        }
    };
    let application = wide_os(helper_exe.as_os_str());
    let mut command_line = helper_command_line(&helper_exe, state_path, helper);
    let current_directory = exe
        .parent()
        .map(|path| wide_os(path.as_os_str()))
        .unwrap_or_else(|| vec![0]);
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = attributes.pointer;
    let mut process = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            current_directory.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        let error = last_error("클램셸 감시자 실행 실패");
        drop(helper_file);
        let _ = std::fs::remove_file(&helper_exe);
        return Err(error);
    }
    let process_handle = OwnedHandle(process.hProcess);
    let thread_handle = OwnedHandle(process.hThread);
    drop(thread_handle);
    let handles = [ready.0, process_handle.0];
    let wait = unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, 5_000) };
    if wait == WAIT_OBJECT_0 {
        drop(helper_file);
        Ok(())
    } else {
        let mut error = if wait == WAIT_TIMEOUT {
            "클램셸 감시자가 준비되지 않았습니다".to_string()
        } else if wait == WAIT_OBJECT_0 + 1 {
            "클램셸 감시자가 준비 전에 종료됐습니다".to_string()
        } else {
            last_error("클램셸 감시자 준비 대기 실패")
        };
        if wait != WAIT_OBJECT_0 + 1 && unsafe { TerminateProcess(process_handle.0, 1) } == 0 {
            error.push_str("; 감시자 종료 실패");
        }
        unsafe { WaitForSingleObject(process_handle.0, 5_000) };
        drop(helper_file);
        let _ = std::fs::remove_file(&helper_exe);
        Err(error)
    }
}

fn helper_main(state_path: PathBuf, helper: String) -> Result<(), String> {
    let owner = NamedOwner::acquire()?;
    let mutex_name = helper_mutex_name(&helper);
    let mutex_wide = wide(&mutex_name);
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, mutex_wide.as_ptr()) };
    if mutex.is_null() {
        return Err(last_error("클램셸 감시자 잠금 생성 실패"));
    }
    let mutex = OwnedHandle(mutex);
    if unsafe { windows_sys::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err("같은 클램셸 감시자가 이미 실행 중입니다".into());
    }
    let Some(state) = read_state(&state_path)? else {
        return Err("클램셸 상태가 사라졌습니다".into());
    };
    if state.helper != helper {
        return Err("클램셸 감시자 식별자가 바뀌었습니다".into());
    }
    let request = RequestGuard::acquire(&state_path)?;
    let runtime = HelperRuntime {
        state_path,
        helper: helper.clone(),
        fallback: state,
        seen_closed: false,
        once_restore_pending: false,
        fail_safe_restore_pending: false,
        request,
    };

    let class_name = wide("SwitcherClamshellWindowsHelper-v1");
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class = WNDCLASSW {
        lpfnWndProc: Some(helper_window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(last_error("클램셸 감시자 창 클래스 생성 실패"));
    }
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(last_error("클램셸 감시자 창 생성 실패"));
    }
    let lid = unsafe {
        RegisterPowerSettingNotification(
            hwnd as HANDLE,
            &GUID_LIDSWITCH_STATE_CHANGE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    let scheme = unsafe {
        RegisterPowerSettingNotification(
            hwnd as HANDLE,
            &GUID_ACTIVE_POWERSCHEME,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    let lid_action = unsafe {
        RegisterPowerSettingNotification(
            hwnd as HANDLE,
            &GUID_LIDCLOSE_ACTION,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    if lid == 0
        || scheme == 0
        || lid_action == 0
        || unsafe { SetTimer(hwnd, TIMER_ID, 500, None) } == 0
    {
        if lid != 0 {
            unsafe { UnregisterPowerSettingNotification(lid) };
        }
        if scheme != 0 {
            unsafe { UnregisterPowerSettingNotification(scheme) };
        }
        if lid_action != 0 {
            unsafe { UnregisterPowerSettingNotification(lid_action) };
        }
        unsafe { DestroyWindow(hwnd) };
        return Err(last_error("클램셸 전원 알림 등록 실패"));
    }
    HELPER
        .set(Mutex::new(runtime))
        .map_err(|_| "클램셸 감시자 상태 초기화 실패".to_string())?;
    if let Err(error) = signal_ready(&helper) {
        if let Some(runtime) = HELPER.get() {
            if let Ok(mut runtime) = runtime.lock() {
                runtime.request.clear();
            }
        }
        unsafe {
            UnregisterPowerSettingNotification(lid);
            UnregisterPowerSettingNotification(scheme);
            UnregisterPowerSettingNotification(lid_action);
            DestroyWindow(hwnd);
        }
        return Err(error);
    }

    let mut message: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe { DispatchMessageW(&message) };
    }
    unsafe {
        UnregisterPowerSettingNotification(lid);
        UnregisterPowerSettingNotification(scheme);
        UnregisterPowerSettingNotification(lid_action);
    }
    if let Some(runtime) = HELPER.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.request.clear();
        }
    }
    drop(mutex);
    drop(owner);
    Ok(())
}

struct HelperRuntime {
    state_path: PathBuf,
    helper: String,
    fallback: State,
    seen_closed: bool,
    once_restore_pending: bool,
    fail_safe_restore_pending: bool,
    request: RequestGuard,
}

impl HelperRuntime {
    fn current_state(&mut self) -> Result<Option<State>, String> {
        if intentionally_stopped(&self.state_path, &self.helper) {
            return Ok(None);
        }
        match read_state(&self.state_path) {
            Ok(Some(state)) if state.helper == self.helper => {
                self.fallback = state.clone();
                if !matches!(read_state_file(&self.state_path), Ok(Some(_))) {
                    write_state(&self.state_path, &state)?;
                }
                Ok(Some(state))
            }
            Ok(Some(_)) => Ok(None),
            Ok(None) | Err(_) => {
                write_state(&self.state_path, &self.fallback)?;
                Ok(Some(self.fallback.clone()))
            }
        }
    }

    fn active_scheme_changed(&mut self) -> Result<bool, String> {
        let _named = NamedOperation::lock(&self.helper)?;
        let Some(mut state) = (match self.current_state() {
            Ok(state) => state,
            Err(error) => return self.begin_fail_safe_restore(error),
        }) else {
            self.request.clear();
            return Ok(true);
        };
        if state.mode == 3 {
            self.fallback = state;
            return self.begin_fail_safe_restore("중단된 클램셸 복원 재개".into());
        }
        match apply_active(&mut self.request.backend, &self.state_path, &mut state) {
            Ok(()) => {
                self.fallback = state;
                Ok(false)
            }
            Err(error) => {
                self.fallback = state;
                self.begin_fail_safe_restore(error)
            }
        }
    }

    fn lid_changed(&mut self, open: bool) -> Result<bool, String> {
        if !open {
            self.seen_closed = true;
            return Ok(false);
        }
        if !self.seen_closed {
            return Ok(false);
        }
        self.once_restore_pending = true;
        self.retry_once_restore()
    }

    fn retry_once_restore(&mut self) -> Result<bool, String> {
        let _named = NamedOperation::lock(&self.helper)?;
        let Some(state) = (match self.current_state() {
            Ok(state) => state,
            Err(error) => return self.begin_fail_safe_restore(error),
        }) else {
            self.request.clear();
            return Ok(true);
        };
        if state.mode != 1 {
            self.once_restore_pending = false;
            return Ok(false);
        }
        restore_all(&mut self.request.backend, &self.state_path, &state)?;
        self.once_restore_pending = false;
        self.request.clear();
        Ok(true)
    }

    fn tick(&mut self) -> Result<bool, String> {
        if self.fail_safe_restore_pending {
            let _named = NamedOperation::lock(&self.helper)?;
            return self.retry_fail_safe_restore();
        }
        if self.once_restore_pending {
            return match self.retry_once_restore() {
                Ok(should_quit) => Ok(should_quit),
                Err(error) => {
                    eprintln!("일회성 클램셸 복원 재시도 대기: {error}");
                    Ok(false)
                }
            };
        }
        self.active_scheme_changed()
    }

    fn begin_fail_safe_restore(&mut self, cause: String) -> Result<bool, String> {
        self.fail_safe_restore_pending = true;
        match self.retry_fail_safe_restore() {
            Ok(done) => Ok(done),
            Err(restore) => Err(format!("{cause}; 메모리 복구본 복원도 실패: {restore}")),
        }
    }

    fn retry_fail_safe_restore(&mut self) -> Result<bool, String> {
        let state = self.fallback.clone();
        match restore_all(&mut self.request.backend, &self.state_path, &state) {
            Ok(()) => {
                self.request.clear();
                self.fail_safe_restore_pending = false;
                Ok(true)
            }
            Err(error) => {
                // restore_all clears the system request even when journal cleanup or
                // one of the power writes fails. Keep the helper and its only known
                // good copy alive, and never re-enter apply_active while retrying.
                self.request.clear();
                Err(error)
            }
        }
    }

    fn shutdown(&mut self) -> Result<(), String> {
        let state = match self.current_state() {
            Ok(Some(state)) => state,
            Ok(None) => {
                self.request.clear();
                return Ok(());
            }
            Err(error) => {
                let fallback = self.fallback.clone();
                let restore = restore_all(&mut self.request.backend, &self.state_path, &fallback);
                self.request.clear();
                return restore.map_err(|restore| {
                    format!("{error}; 종료 중 메모리 복구본 복원도 실패: {restore}")
                });
            }
        };
        let result = restore_all(&mut self.request.backend, &self.state_path, &state);
        self.request.clear();
        result
    }
}

unsafe extern "system" fn helper_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }

    let mut quit = false;
    let mut handled = None;
    if let Some(runtime) = HELPER.get() {
        if let Ok(mut runtime) = runtime.lock() {
            match message {
                WM_POWERBROADCAST if wparam as u32 == PBT_POWERSETTINGCHANGE => {
                    let setting = lparam as *const POWERBROADCAST_SETTING;
                    if !setting.is_null() {
                        if guid_eq(&(*setting).PowerSetting, &GUID_LIDSWITCH_STATE_CHANGE)
                            && (*setting).DataLength >= 4
                        {
                            let value: u32 =
                                std::ptr::read_unaligned((*setting).Data.as_ptr().cast());
                            match runtime.lid_changed(value != 0) {
                                Ok(should_quit) => quit = should_quit,
                                Err(error) => {
                                    eprintln!("일회성 클램셸 복원 실패, 재시도 예정: {error}");
                                }
                            }
                        } else if guid_eq(&(*setting).PowerSetting, &GUID_ACTIVE_POWERSCHEME)
                            || guid_eq(&(*setting).PowerSetting, &GUID_LIDCLOSE_ACTION)
                        {
                            match runtime.active_scheme_changed() {
                                Ok(should_quit) => quit = should_quit,
                                Err(error) => {
                                    eprintln!("새 전원 관리 옵션 적용 실패: {error}");
                                }
                            }
                        }
                    }
                    handled = Some(1);
                }
                WM_TIMER if wparam == TIMER_ID => {
                    match runtime.tick() {
                        Ok(should_quit) => quit = should_quit,
                        Err(error) => {
                            eprintln!("클램셸 상태 점검 실패: {error}");
                        }
                    }
                    handled = Some(0);
                }
                WM_QUERYENDSESSION => {
                    handled = Some(1);
                }
                WM_ENDSESSION if wparam != 0 => {
                    if let Err(error) = runtime.shutdown() {
                        eprintln!("종료 중 클램셸 복원 실패: {error}");
                    }
                    quit = true;
                    handled = Some(0);
                }
                _ => {}
            }
        }
    }
    if quit {
        DestroyWindow(hwnd);
    }
    if let Some(result) = handled {
        return result;
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

fn helper_alive(helper: &str) -> bool {
    if !valid_token(helper) {
        return false;
    }
    let name = wide(&helper_mutex_name(helper));
    let handle = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    let owned = OwnedHandle(handle);
    (unsafe { WaitForSingleObject(owned.0, 0) }) == WAIT_TIMEOUT
}

fn signal_ready(helper: &str) -> Result<(), String> {
    let name = wide(&ready_name(helper));
    let handle = unsafe {
        windows_sys::Win32::System::Threading::OpenEventW(
            windows_sys::Win32::System::Threading::EVENT_MODIFY_STATE,
            0,
            name.as_ptr(),
        )
    };
    if handle.is_null() {
        return Err(last_error("클램셸 준비 이벤트 열기 실패"));
    }
    let handle = OwnedHandle(handle);
    if unsafe { SetEvent(handle.0) } == 0 {
        Err(last_error("클램셸 준비 신호 실패"))
    } else {
        Ok(())
    }
}

fn helper_mutex_name(helper: &str) -> String {
    format!("Global\\SwitcherClamshellWindowsHelper-{helper}")
}

fn operation_mutex_name(helper: &str) -> String {
    format!("Global\\SwitcherClamshellWindowsOperation-{helper}")
}

fn ready_name(helper: &str) -> String {
    format!("Global\\SwitcherClamshellWindowsReady-{helper}")
}

fn helper_copy_path(state_path: &Path, helper: &str) -> Result<PathBuf, String> {
    if !valid_token(helper) {
        return Err("클램셸 감시자 식별자가 올바르지 않습니다".into());
    }
    let parent = state_path.parent().ok_or("클램셸 상태 폴더가 없습니다")?;
    Ok(parent.join(format!("{HELPER_FILE_PREFIX}{helper}.exe")))
}

fn cleanup_stale_helpers(store: &Path, preserve: Option<&str>) {
    let preserve = preserve.and_then(|helper| helper_copy_path(&files(store), helper).ok());
    let Ok(entries) = std::fs::read_dir(store) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if preserve.as_ref() == Some(&path) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with(HELPER_FILE_PREFIX) && name.ends_with(".exe") {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn win32(status: u32, context: &str) -> Result<(), String> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("{context} (Windows 오류 {status})"))
    }
}

fn last_error(context: &str) -> String {
    let status = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    format!("{context} (Windows 오류 {status})")
}

fn combine_rollback(error: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => error,
        Err(rollback) => format!("{error}; 되돌리기도 실패: {rollback}"),
    }
}

fn combine_rollbacks(error: String, ac: Result<(), String>, dc: Result<(), String>) -> String {
    let mut message = error;
    for rollback in [ac, dc].into_iter().filter_map(Result::err) {
        message.push_str("; 되돌리기도 실패: ");
        message.push_str(&rollback);
    }
    message
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn guid_text(guid: &GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

fn guid_eq(left: &GUID, right: &GUID) -> bool {
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}

fn parse_guid(value: &str) -> Result<GUID, String> {
    let compact: String = value.chars().filter(|ch| *ch != '-').collect();
    if compact.len() != 32 || !compact.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("전원 관리 옵션 GUID 형식 오류".into());
    }
    let number = u128::from_str_radix(&compact, 16)
        .map_err(|_| "전원 관리 옵션 GUID 형식 오류".to_string())?;
    Ok(GUID::from_u128(number))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const A: &str = "11111111-1111-1111-1111-111111111111";
    const B: &str = "22222222-2222-2222-2222-222222222222";

    #[derive(Default)]
    struct FakePower {
        active: String,
        values: HashMap<String, (u32, u32)>,
        calls: Vec<String>,
        fail: Option<String>,
        switch_on_apply: Option<String>,
        request: bool,
    }

    impl FakePower {
        fn with_scheme(name: &str, ac: u32, dc: u32) -> Self {
            let mut values = HashMap::new();
            values.insert(name.into(), (ac, dc));
            Self {
                active: name.into(),
                values,
                ..Self::default()
            }
        }

        fn maybe_fail(&self, call: &str) -> Result<(), String> {
            if self.fail.as_deref() == Some(call) {
                Err(format!("failed {call}"))
            } else {
                Ok(())
            }
        }
    }

    impl PowerBackend for FakePower {
        fn has_lid(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn check_access(&mut self) -> Result<(), String> {
            let call = "check-access".to_string();
            self.calls.push(call.clone());
            self.maybe_fail(&call)
        }
        fn active_scheme(&mut self) -> Result<String, String> {
            Ok(self.active.clone())
        }
        fn scheme_exists(&mut self, scheme: &str) -> Result<bool, String> {
            Ok(self.values.contains_key(scheme))
        }
        fn read_actions(&mut self, scheme: &str) -> Result<(u32, u32), String> {
            self.values
                .get(scheme)
                .copied()
                .ok_or_else(|| "missing scheme".into())
        }
        fn write_ac(&mut self, scheme: &str, value: u32) -> Result<(), String> {
            let call = format!("ac:{scheme}:{value}");
            self.calls.push(call.clone());
            self.maybe_fail(&call)?;
            self.values.get_mut(scheme).unwrap().0 = value;
            Ok(())
        }
        fn write_dc(&mut self, scheme: &str, value: u32) -> Result<(), String> {
            let call = format!("dc:{scheme}:{value}");
            self.calls.push(call.clone());
            self.maybe_fail(&call)?;
            self.values.get_mut(scheme).unwrap().1 = value;
            Ok(())
        }
        fn apply_current_lid_settings(&mut self) -> Result<(), String> {
            if let Some(next) = self.switch_on_apply.take() {
                self.active = next;
            }
            let call = "apply-current".to_string();
            self.calls.push(call.clone());
            self.maybe_fail(&call)
        }
        fn request_system(&mut self) -> Result<(), String> {
            self.calls.push("request".into());
            self.request = true;
            Ok(())
        }
        fn clear_request(&mut self) {
            self.calls.push("clear".into());
            self.request = false;
        }
    }

    fn temp_state(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "switcher-clamshell-test-{name}-{}-{}",
            std::process::id(),
            token().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(STATE_FILE)
    }

    fn state(mode: u8, schemes: Vec<SchemeJournal>) -> State {
        State {
            version: 1,
            mode,
            revision: "revision-1".into(),
            helper: "helper-1".into(),
            schemes,
        }
    }

    #[test]
    fn journal_is_written_before_ac_and_dc_changes() {
        let path = temp_state("journal-order");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut backend = FakePower::with_scheme(A, 1, 2);
        let mut current = state(1, Vec::new());

        apply_active(&mut backend, &path, &mut current).unwrap();

        let persisted = read_state(&path).unwrap().unwrap();
        assert_eq!(
            persisted.schemes[0],
            SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2
            }
        );
        assert_eq!(backend.values[A], (0, 0));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn apply_preflight_failure_never_writes_a_journal_or_power_value() {
        let path = temp_state("apply-preflight");
        let mut backend = FakePower::with_scheme(A, 1, 2);
        backend.fail = Some("check-access".into());
        let mut current = state(1, Vec::new());

        assert!(activate(&mut backend, &path, &mut current).is_err());

        assert_eq!(backend.values[A], (1, 2));
        assert_eq!(backend.calls, ["check-access"]);
        assert!(!path.exists());
        assert!(!recovery_file(&path).unwrap().exists());
    }

    #[test]
    fn full_zero_one_two_zero_cycle_restores_exact_settings() {
        let path = temp_state("full-cycle");
        let mut backend = FakePower::with_scheme(A, 1, 3);
        let mut current = state(1, Vec::new());

        apply_active(&mut backend, &path, &mut current).unwrap();
        assert_eq!(read_state(&path).unwrap().unwrap().mode, 1);
        assert_eq!(backend.values[A], (0, 0));

        current.mode = 2;
        current.revision = "revision-2".into();
        write_state(&path, &current).unwrap();
        assert_eq!(read_state(&path).unwrap().unwrap().mode, 2);
        assert_eq!(current.schemes.len(), 1);

        restore_all(&mut backend, &path, &current).unwrap();
        assert_eq!(backend.values[A], (1, 3));
        assert!(!path.exists());
    }

    #[test]
    fn recovery_copy_survives_primary_journal_corruption() {
        let path = temp_state("recovery-copy");
        let journal = state(
            2,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 3,
            }],
        );
        write_state(&path, &journal).unwrap();
        std::fs::write(&path, b"{").unwrap();

        assert_eq!(read_state(&path).unwrap(), Some(journal.clone()));

        remove_state(&path, &journal.helper).unwrap();
        let _ = remove_file_if_present(&stop_file(&path).unwrap(), "cleanup");
    }

    #[test]
    fn missing_journal_is_repaired_from_live_helper_fallback() {
        let path = temp_state("live-fallback");
        let journal = state(
            1,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2,
            }],
        );
        let request = RequestGuard {
            backend: NativePower,
            held: false,
        };
        let mut runtime = HelperRuntime {
            state_path: path.clone(),
            helper: journal.helper.clone(),
            fallback: journal.clone(),
            seen_closed: false,
            once_restore_pending: false,
            fail_safe_restore_pending: false,
            request,
        };

        assert_eq!(runtime.current_state().unwrap(), Some(journal.clone()));
        assert_eq!(read_state(&path).unwrap(), Some(journal.clone()));
        remove_state(&path, &journal.helper).unwrap();
        assert_eq!(runtime.current_state().unwrap(), None);
        let _ = remove_file_if_present(&stop_file(&path).unwrap(), "cleanup");
    }

    #[test]
    fn stop_marker_wins_over_a_lingering_valid_journal() {
        let path = temp_state("stop-wins");
        let journal = state(
            2,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2,
            }],
        );
        write_state(&path, &journal).unwrap();
        crate::accounts::atomic_write_existing_parent(
            &stop_file(&path).unwrap(),
            journal.helper.as_bytes(),
        )
        .unwrap();
        let request = RequestGuard {
            backend: NativePower,
            held: false,
        };
        let mut runtime = HelperRuntime {
            state_path: path.clone(),
            helper: journal.helper.clone(),
            fallback: journal.clone(),
            seen_closed: false,
            once_restore_pending: false,
            fail_safe_restore_pending: false,
            request,
        };

        assert_eq!(runtime.current_state().unwrap(), None);
        assert_eq!(mode(path.parent().unwrap()), 0);

        let _ = remove_file_if_present(&path, "cleanup");
        let _ = remove_file_if_present(&recovery_file(&path).unwrap(), "cleanup");
        let _ = remove_file_if_present(&stop_file(&path).unwrap(), "cleanup");
    }

    #[test]
    fn helper_command_line_quotes_spaces_and_trailing_backslashes() {
        let command = helper_command_line(
            Path::new(r"C:\Program Files\Switcher\switcher.exe"),
            Path::new(r"C:\Users\test name\.switcher\state.json"),
            "helper-1",
        );
        let text = String::from_utf16(&command[..command.len() - 1]).unwrap();
        assert_eq!(
            text,
            r#""C:\Program Files\Switcher\switcher.exe" "--switcher-clamshell-windows-helper" "C:\Users\test name\.switcher\state.json" "helper-1""#
        );

        let mut quoted = Vec::new();
        quote_windows_arg(std::ffi::OsStr::new(r"C:\trailing\"), &mut quoted);
        assert_eq!(String::from_utf16(&quoted).unwrap(), r#""C:\trailing\\""#);
    }

    #[test]
    fn dc_failure_rolls_ac_back() {
        let mut backend = FakePower::with_scheme(A, 1, 2);
        backend.fail = Some(format!("dc:{A}:0"));

        assert!(ensure_ignored(&mut backend, A, 1, 2).is_err());
        assert_eq!(backend.values[A], (1, 2));
        assert_eq!(
            backend.calls,
            [
                format!("ac:{A}:0"),
                format!("dc:{A}:0"),
                format!("ac:{A}:1")
            ]
        );
    }

    #[test]
    fn activation_failure_rolls_ac_and_dc_back() {
        let mut backend = FakePower::with_scheme(A, 1, 2);
        backend.fail = Some("apply-current".into());

        assert!(ensure_ignored(&mut backend, A, 1, 2).is_err());
        assert_eq!(backend.values[A], (1, 2));
    }

    #[test]
    fn scheme_change_during_apply_never_reactivates_old_scheme() {
        let mut backend = FakePower::with_scheme(A, 1, 2);
        backend.values.insert(B.into(), (3, 1));
        backend.switch_on_apply = Some(B.into());

        ensure_ignored(&mut backend, A, 1, 2).unwrap();

        assert_eq!(backend.active, B);
        assert_eq!(backend.values[A], (0, 0));
        assert!(backend.calls.contains(&"apply-current".to_string()));
    }

    #[test]
    fn scheme_changes_are_journaled_once_and_all_restore_without_switching() {
        let path = temp_state("schemes");
        let mut backend = FakePower::with_scheme(A, 1, 2);
        backend.values.insert(B.into(), (3, 1));
        let mut current = state(2, Vec::new());
        apply_active(&mut backend, &path, &mut current).unwrap();
        backend.active = B.into();
        apply_active(&mut backend, &path, &mut current).unwrap();
        apply_active(&mut backend, &path, &mut current).unwrap();

        restore_all(&mut backend, &path, &current).unwrap();

        assert_eq!(backend.values[A], (1, 2));
        assert_eq!(backend.values[B], (3, 1));
        assert_eq!(backend.active, B);
        assert_eq!(current.schemes.len(), 2);
    }

    #[test]
    fn deleted_scheme_does_not_block_restoring_live_schemes_or_turning_off() {
        let path = temp_state("deleted-scheme");
        let journal = state(
            2,
            vec![
                SchemeJournal {
                    scheme: A.into(),
                    ac: 1,
                    dc: 2,
                },
                SchemeJournal {
                    scheme: B.into(),
                    ac: 3,
                    dc: 1,
                },
            ],
        );
        write_state(&path, &journal).unwrap();
        let mut backend = FakePower::with_scheme(B, 0, 0);

        restore_all(&mut backend, &path, &journal).unwrap();

        assert_eq!(backend.values[B], (3, 1));
        assert!(!path.exists());
        assert!(!backend.calls.iter().any(|call| call.contains(A)));
    }

    #[test]
    fn same_scheme_external_edit_is_reapplied_without_duplicate_journal() {
        let path = temp_state("same-scheme-edit");
        let mut backend = FakePower::with_scheme(A, 1, 2);
        let mut current = state(2, Vec::new());
        apply_active(&mut backend, &path, &mut current).unwrap();
        backend.values.insert(A.into(), (3, 1));

        apply_active(&mut backend, &path, &mut current).unwrap();

        assert_eq!(backend.values[A], (0, 0));
        assert_eq!(current.schemes.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn zero_baseline_is_noop_across_chained_helper_recovery() {
        let first_path = temp_state("first-owner");
        let second_path = temp_state("second-noop");
        let first = state(
            2,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2,
            }],
        );
        let second = state(
            2,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 0,
                dc: 0,
            }],
        );
        write_state(&first_path, &first).unwrap();
        write_state(&second_path, &second).unwrap();
        let mut backend = FakePower::with_scheme(A, 0, 0);

        restore_all(&mut backend, &first_path, &first).unwrap();
        restore_all(&mut backend, &second_path, &second).unwrap();

        assert_eq!(backend.values[A], (1, 2));
    }

    #[test]
    fn zero_baseline_claims_a_later_nonzero_value_before_suppressing_it() {
        let path = temp_state("claim-later-value");
        let mut current = state(
            2,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 0,
                dc: 0,
            }],
        );
        write_state(&path, &current).unwrap();
        let mut backend = FakePower::with_scheme(A, 3, 2);

        apply_active(&mut backend, &path, &mut current).unwrap();

        assert_eq!(current.schemes[0].ac, 3);
        assert_eq!(current.schemes[0].dc, 2);
        assert_eq!(read_state(&path).unwrap(), Some(current.clone()));
        assert_eq!(backend.values[A], (0, 0));
        restore_all(&mut backend, &path, &current).unwrap();
        assert_eq!(backend.values[A], (3, 2));
    }

    #[test]
    fn helper_copy_is_separate_from_installed_executable() {
        let state_path = Path::new(r"C:\Users\tester\.switcher\clamshell-windows-state.json");
        let installed = Path::new(r"C:\Program Files\Switcher\switcher.exe");

        let helper = helper_copy_path(state_path, "helper-1").unwrap();

        assert_ne!(helper, installed);
        assert_eq!(
            helper,
            Path::new(r"C:\Users\tester\.switcher\clamshell-windows-helper-helper-1.exe")
        );
    }

    #[test]
    fn verified_read_lock_allows_the_helper_copy_to_start() {
        use std::os::windows::process::CommandExt;

        let source = PathBuf::from(
            std::env::var_os("COMSPEC").expect("Windows must expose COMSPEC for this test"),
        );
        let state_path = temp_state("helper-launch-lock");
        let destination = helper_copy_path(&state_path, "helper-1").unwrap();
        create_helper_copy(&source, &destination).unwrap();
        let launch_lock = open_verified_helper_copy(&source, &destination).unwrap();

        let status = std::process::Command::new(&destination)
            .args(["/d", "/c", "exit 0"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .unwrap();

        assert!(status.success());
        drop(launch_lock);
        std::fs::remove_file(&destination).unwrap();
        let _ = std::fs::remove_dir(state_path.parent().unwrap());
    }

    #[test]
    fn tampered_helper_copy_is_rejected_before_launch() {
        use std::io::Write;

        let source = PathBuf::from(
            std::env::var_os("COMSPEC").expect("Windows must expose COMSPEC for this test"),
        );
        let state_path = temp_state("helper-copy-tamper");
        let destination = helper_copy_path(&state_path, "helper-1").unwrap();
        create_helper_copy(&source, &destination).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&destination)
            .unwrap()
            .write_all(&[0])
            .unwrap();

        assert!(open_verified_helper_copy(&source, &destination).is_err());

        std::fs::remove_file(&destination).unwrap();
        let _ = std::fs::remove_dir(state_path.parent().unwrap());
    }

    #[test]
    fn generated_tokens_are_valid_and_distinct() {
        let first = token().unwrap();
        let second = token().unwrap();
        assert!(valid_token(&first));
        assert!(valid_token(&second));
        assert_ne!(first, second);
    }

    #[test]
    fn initial_open_does_not_restore_and_once_to_keep_wins_open_race() {
        let mut backend = FakePower::with_scheme(A, 0, 0);
        let path = temp_state("lid-semantics");
        let journal = state(
            1,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2,
            }],
        );
        write_state(&path, &journal).unwrap();
        let request = RequestGuard {
            backend: NativePower,
            held: false,
        };
        let mut runtime = HelperRuntime {
            state_path: path.clone(),
            helper: journal.helper.clone(),
            fallback: journal.clone(),
            seen_closed: false,
            once_restore_pending: false,
            fail_safe_restore_pending: false,
            request,
        };

        assert!(!runtime.lid_changed(true).unwrap());
        assert!(path.exists());
        runtime.seen_closed = true;
        let mut promoted = journal.clone();
        promoted.mode = 2;
        write_state(&path, &promoted).unwrap();
        assert!(!runtime.retry_once_restore().unwrap());
        assert!(path.exists());

        backend.clear_request();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dead_helper_recovery_restores_and_live_helper_is_preserved() {
        let path = temp_state("recovery");
        let journal = state(
            2,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2,
            }],
        );
        write_state(&path, &journal).unwrap();
        let mut backend = FakePower::with_scheme(A, 0, 0);

        assert!(!recover_dead(&mut backend, &path, &journal, true).unwrap());
        assert!(path.exists());
        assert!(recover_dead(&mut backend, &path, &journal, false).unwrap());
        assert_eq!(backend.values[A], (1, 2));
        assert!(!path.exists());
    }

    #[test]
    fn restore_failure_retains_journal_and_clears_request() {
        let path = temp_state("retain");
        let journal = state(
            1,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2,
            }],
        );
        write_state(&path, &journal).unwrap();
        let mut backend = FakePower::with_scheme(A, 0, 0);
        backend.request = true;
        backend.fail = Some(format!("dc:{A}:2"));

        assert!(restore_all(&mut backend, &path, &journal).is_err());
        assert!(path.exists());
        assert!(!backend.request);
    }

    #[test]
    fn system_request_is_acquired_after_journal_and_cleared_on_rollback() {
        let path = temp_state("request");
        let journal = state(
            1,
            vec![SchemeJournal {
                scheme: A.into(),
                ac: 1,
                dc: 2,
            }],
        );
        let mut backend = FakePower::with_scheme(A, 0, 0);

        assert!(acquire_request_after_journal(&mut backend, &path).is_err());
        assert!(!backend.request);
        assert!(!backend.calls.iter().any(|call| call == "request"));
        write_state(&path, &journal).unwrap();
        acquire_request_after_journal(&mut backend, &path).unwrap();
        assert!(backend.request);
        restore_all(&mut backend, &path, &journal).unwrap();
        assert!(!backend.request);
    }
}
