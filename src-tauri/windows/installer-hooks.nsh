; ForgeDesk NSIS 安装器钩子（T0.11）。
;
; Tauri 的 NSIS 模板在安装/卸载的关键节点调用这些宏（名字必须与模板约定一致）。
; 这里只做一件事：注册/注销 forgedesk:// 自定义协议，为 M8 的深链接预留。
;
; 设计说明：
; - **只注册、不处理**：URL 到达应用后如何路由（打开仓库、唤起面板）属于 M8 的
;   深链接处理逻辑；现在注册协议是安全的——没有处理逻辑时，点击链接只会启动应用。
; - **写在 HKCU**：installMode = currentUser 时 SHCTX 指向当前用户，
;   不需要管理员权限，与"免 UAC 的 per-user 安装"一致；
;   per-machine 的 MSI 不注册该协议（见 tauri.conf.json 与提交说明的理由）。
; - 卸载时必须清理，否则卸载后点击 forgedesk:// 链接会指向不存在的程序。

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\Classes\forgedesk" "" "URL:ForgeDesk Protocol"
  WriteRegStr SHCTX "Software\Classes\forgedesk" "URL Protocol" ""
  WriteRegStr SHCTX "Software\Classes\forgedesk\DefaultIcon" "" "$INSTDIR\ForgeDesk.exe,0"
  WriteRegStr SHCTX "Software\Classes\forgedesk\shell\open\command" "" '"$INSTDIR\ForgeDesk.exe" "%1"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 先删子键再删父键（DeleteRegKey /R 会连子键一起删，这里用 /R 一次完成）
  DeleteRegKey SHCTX "Software\Classes\forgedesk\shell\open\command"
  DeleteRegKey SHCTX "Software\Classes\forgedesk\shell\open"
  DeleteRegKey SHCTX "Software\Classes\forgedesk\shell"
  DeleteRegKey SHCTX "Software\Classes\forgedesk\DefaultIcon"
  DeleteRegKey SHCTX "Software\Classes\forgedesk"
!macroend
