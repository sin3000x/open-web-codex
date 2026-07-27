#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
npm --prefix "$repo_root/apps/web" test -- \
  src/utils/inlineVisualizations.test.ts \
  src/utils/replyCards.test.ts \
  src/utils/webThreadHistory.test.ts \
  src/components/Conversation/MessageList.test.tsx \
  src/components/Conversation/messages/MapReplyCard.test.ts \
  --run
