# Chat UI Refresh & Vision Error Handling

## Goal
Redesign the entire Chat page UI to a modern, polished look and implement comprehensive image/vision error handling with pre-send validation, inline attachment warnings, and improved error messages.

## Key Files

### Frontend
- `apps/web/src/features/chat/Chat.tsx` — Main chat component (messages, composer, attachments)
- `apps/web/src/components/shared.tsx` — ErrorBox component
- `apps/web/src/components/button.tsx` — Button component
- `apps/web/src/features/chat/ArtifactCard.tsx` — Artifact display
- `apps/web/src/features/chat/ModeStatus.tsx` — Mode/budget status display
- `apps/web/src/style.css` — All styles (~5923 lines)

### i18n
- `apps/web/src/app/i18n.ts` — Translations (ja/en)

## Part 1: Chat Component Logic Changes (Chat.tsx)

### 1a. Vision capability tracking
Add a computed boolean that tracks whether the currently selected model supports vision:
```typescript
const selectedModelCaps = { ...selected?.data.capabilities, ...selected?.data.overrides };
const modelSupportsVision = selectedModelCaps.vision === true;
```

### 1b. Pre-send validation for images + non-vision models
In the `send()` function, before the API call, check if any attachments are images AND the model doesn't support vision. If so, set an error and return early:
```typescript
const hasImage = attachments.some(a => a.mime?.startsWith("image/"));
if (hasImage && model !== "auto" && !modelSupportsVision) {
  setError(new Error("画像を添付するにはVision対応モデルが必要です"));
  return;
}
```

### 1c. Attachment warning state
Track whether attachments contain images that the current model can't process:
```typescript
const imageAttachmentWarning = !modelSupportsVision && attachments.some(a => a.mime?.startsWith("image/"));
```

### 1d. Enhanced ErrorBox with action button
When the error is vision-related, show a hint to switch models:
```typescript
const isVisionError = error instanceof Error && error.message.includes("Vision");
```

### 1e. New attachment chip design
- Show image thumbnail preview for image files (using object URL)
- Add a yellow warning badge on image chips when model doesn't support vision
- Better file type icons

## Part 2: Shared Components Updates

### 2a. ErrorBox enhancement (shared.tsx)
Add optional `variant` prop to support different error styles:
- `"default"` — current red style
- `"warning"` — yellow/amber style for pre-send warnings
- Add optional `action` prop with a button label + onClick for actionable errors (e.g., "モデルを変更")

```tsx
export function ErrorBox({ error, variant = "default", action, onAction }: {
  error: unknown;
  variant?: "default" | "warning";
  action?: string;
  onAction?: () => void;
}) { ... }
```

## Part 3: CSS Changes (style.css)

### 3a. Composer refresh
- Increase border-radius to 24px, add subtle gradient border
- Better focus state with accent glow ring
- Larger, more comfortable textarea with smoother placeholder
- Animate send button with scale effect

### 3b. Attachment chips
- Modern pill design with backdrop-filter blur
- Image thumbnail preview (40x40px rounded)
- Warning badge (yellow triangle icon) on incompatible image chips
- Smooth enter/exit animation

### 3c. Error box
- Add `.error.warning` variant with amber background
- Action button styling for inline model switch prompt
- Icon prefix (alert-circle for errors, alert-triangle for warnings)
- Better padding and line-height

### 3d. Message bubbles
- Assistant messages: add subtle left border accent
- User messages: refined gradient background
- Better code block styling with syntax-like header
- Smoother performance badge

### 3e. Welcome screen
- Refined spacing and typography
- Subtle gradient on welcome icon
- Better eyebrow text treatment

## Part 4: i18n Strings

Add new translations to `i18n.ts`:

Japanese:
- "画像を添付するにはVision対応モデルを選択してください" → "Please select a Vision model to attach images"
- "このモデルは画像入力に対応していません" → "This model does not support image input"
- "Vision対応モデルに切り替え" → "Switch to Vision model"
- "画像添付は送信されません" → "Image attachments will not be sent"

## Part 5: ModeStatus.tsx Enhancement

Add a vision capability warning when images are attached:
```tsx
{model && !modelSupportsVision && hasImageAttachments && (
  <span className="vision-warning">このモデルは画像を処理できません</span>
)}
```

## Implementation Order

1. Add i18n strings for new messages
2. Update ErrorBox component (shared.tsx) with variant/action support
3. Update Chat.tsx logic (vision tracking, pre-send validation, attachment warning)
4. Update ModeStatus.tsx to show vision warnings
5. Update attachment chip rendering in Chat.tsx (thumbnails, warning badges)
6. Update all CSS in style.css for the new design
7. Test the flow: attach image → see warning → try to send → see error with action

## Verification Checklist
- [ ] Attaching an image to a non-vision model shows warning badge on the chip
- [ ] Trying to send with incompatible image shows error with model switch hint
- [ ] Auto model bypasses the vision check (handled server-side)
- [ ] Vision-compatible models work without warnings
- [ ] Image thumbnails render correctly for various image types
- [ ] Dark mode looks correct for all new elements
- [ ] Responsive layout works at all breakpoints
- [ ] Error dismisses when user removes the image attachment or changes model
