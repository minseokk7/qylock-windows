import sys

file_path = r'src-tauri\src\lib.rs'
with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

content = content.replace('VK_MENU, VK_RWIN, VK_SHIFT, VK_TAB,', 'VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT, VK_TAB,')

verify_hello_str = '''#[tauri::command]
async fn verify_hello(app: AppHandle) -> Result<bool, String> {'''

verify_hello_impl_str = '''async fn verify_hello_impl(app: &AppHandle) -> Result<bool, String> {'''

content = content.replace(verify_hello_str, verify_hello_impl_str)
content = content.replace('set_lock_windows_topmost(&app, false, false);', 'set_lock_windows_topmost(app, false, false);')
content = content.replace('unlock_and_unhook(&app);', 'unlock_and_unhook(app);')
content = content.replace('set_lock_windows_topmost(&app, true, true);', 'set_lock_windows_topmost(app, true, true);')

cmd_str = '''#[tauri::command]
fn emergency_unlock(app: AppHandle) {'''

insert_cmd_str = '''#[tauri::command]
async fn verify_hello(app: AppHandle) -> Result<bool, String> {
    verify_hello_impl(&app).await
}

#[tauri::command]
fn emergency_unlock(app: AppHandle) {'''

content = content.replace(cmd_str, insert_cmd_str)

hook_target = '''        if cfg!(debug_assertions) && is_ctrl && is_alt && is_q {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        if is_locked
            && (is_windows_combo
                || (vk == VK_TAB && is_alt)
                || (vk == VK_F4 && is_alt)
                || (vk == VK_ESCAPE && (is_alt || is_ctrl)))
        {
            return LRESULT(1);
        }
    }'''

hook_replacement = '''        if cfg!(debug_assertions) && is_ctrl && is_alt && is_q {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        if is_locked {
            if is_key_down && vk == VK_RETURN {
                if let Some(app) = APP_HANDLE.get().cloned() {
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = verify_hello_impl(&app).await {
                            eprintln!("Windows Hello invocation from keyboard hook failed: {e}");
                        }
                    });
                }
                return LRESULT(1);
            }

            if is_windows_combo
                || (vk == VK_TAB && is_alt)
                || (vk == VK_F4 && is_alt)
                || (vk == VK_ESCAPE && (is_alt || is_ctrl))
            {
                return LRESULT(1);
            }
        }
    }'''

content = content.replace(hook_target, hook_replacement)
content = content.replace('.transparent(true)', '.transparent(false)')

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)

print("Replace success")
