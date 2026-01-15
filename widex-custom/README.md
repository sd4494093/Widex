# Widex 自定义功能目录

这个目录包含所有 Widex 的自定义功能，与上游代码分离。

## 目录结构

```
widex-custom/
├── models/          # 自定义模型接入（如其他 AI 模型）
├── features/        # 自定义功能模块
├── configs/         # 自定义配置文件
└── docs/            # 自定义文档
```

## 优势

1. **清晰分离**：自定义代码与上游代码分离
2. **易于管理**：所有修改集中在一个目录
3. **减少冲突**：上游更新不太可能影响这个目录
4. **易于维护**：清楚哪些是自定义内容

## 使用示例

### 添加新模型
在 `models/` 目录下创建新的模型适配器：
```
widex-custom/models/
├── gemini/
├── claude/
└── custom_model/
```

### 添加新功能
在 `features/` 目录下创建功能模块：
```
widex-custom/features/
├── enhanced_search/
├── custom_ui/
└── api_extensions/
```
