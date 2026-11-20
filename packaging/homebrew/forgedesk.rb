# Homebrew Cask 清单（真相源：packaging/homebrew/forgedesk.rb）
#
# 发布形态是 **universal** dmg（单一文件同时覆盖 Apple Silicon 与 Intel），
# 因此只有一个 sha256；这与"arm64 + intel 两个 dmg"的旧表述不同（见 README.md §3）。
cask "forgedesk" do
  version "1.1.4"
  sha256 "de789f053caba13226f31dc247e308b031b96079738265bca2b00fed28e60a1c"

  url "https://github.com/Ember1414/forgedesk/releases/download/v#{version}/ForgeDesk_#{version}_macos_universal.dmg",
      verified: "github.com/Ember1414/forgedesk/"
  name "ForgeDesk"
  desc "A Git client."
  homepage "https://forgedesk.pages.dev/"

  # 应用内置自动更新（tauri-plugin-updater，见 docs/RELEASE.md），
  # 因此标记 auto_updates，避免 Homebrew 与内置更新互相打架。
  auto_updates true

  app "ForgeDesk.app"

  zap trash: [
    "~/Library/Application Support/io.github.ember1414.forgedesk",
    "~/Library/Caches/io.github.ember1414.forgedesk",
    "~/Library/Logs/io.github.ember1414.forgedesk",
    "~/Library/Preferences/io.github.ember1414.forgedesk.plist",
    "~/Library/Saved Application State/io.github.ember1414.forgedesk.savedState",
    "~/Library/WebKit/io.github.ember1414.forgedesk",
  ]
end
