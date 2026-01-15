#!/bin/bash
# Widex 上游同步脚本
# 用法: ./sync-upstream.sh

set -e

echo "🔄 开始同步上游更新..."

# 保存当前分支
CURRENT_BRANCH=$(git branch --show-current)

# 切换到 main 并同步
echo "📥 获取上游更新..."
git checkout main
git fetch upstream
git merge upstream/main
git push origin main

# 切换回 widex 并合并
echo "🔀 合并到 widex 分支..."
git checkout widex
git merge main

echo "✅ 同步完成！"
echo "📝 如果有冲突，请解决后执行："
echo "   git add ."
echo "   git commit -m 'Merge upstream updates'"
echo "   git push origin widex"

# 如果之前不在 widex，切回原分支
if [ "$CURRENT_BRANCH" != "widex" ] && [ "$CURRENT_BRANCH" != "main" ]; then
    git checkout "$CURRENT_BRANCH"
fi
