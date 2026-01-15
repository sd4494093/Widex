# Widex 分支管理指南

## 日常使用

**你只需要待在 widex 分支！**

```bash
# 确保在 widex 分支
git checkout widex

# 日常开发
# ... 修改代码 ...
git add .
git commit -m "你的提交信息"
git push origin widex
```

## 同步上游更新

**方式 1：使用脚本（推荐）**
```bash
./sync-upstream.sh
```

**方式 2：手动执行**
```bash
# 1. 更新 main 分支
git checkout main
git fetch upstream
git merge upstream/main
git push origin main

# 2. 合并到 widex
git checkout widex
git merge main
git push origin widex
```

## 分支说明

- **main**: 纯净分支，只用于接收上游更新（很少切换）
- **widex**: 你的工作分支，日常开发都在这里（一直待在这里）

## 何时需要切换分支？

✅ **需要切换的情况（很少）：**
- 同步上游更新时（每周/每月一次）
- 对比原版和定制版差异时

❌ **不需要切换的情况（大部分时间）：**
- 日常开发
- 添加新功能
- 修改配置
- 运行测试

## 快速检查

```bash
# 查看当前分支
git branch

# 查看分支差异
git log main..widex --oneline

# 查看自定义文件
ls widex-custom/
```
