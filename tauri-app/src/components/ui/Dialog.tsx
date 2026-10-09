import * as React from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { cn } from '../../lib/utils'
import { useLocaleStore } from '../../stores/locale-store'

/**
 * `Dialog` = Radix `Root` + 受控关闭上下文。
 *
 * 包一层的目的：把 `onOpenChange(false)` 以 `requestClose()` 的形式下传给
 * `DialogContent`，使其能在 Radix 的 ESC 通道失效时（见 `dialogStack` 说明）
 * 自行兜底关闭。
 */
const Dialog = ({ onOpenChange, open, ...props }: React.ComponentProps<typeof DialogPrimitive.Root>) => {
  const requestClose = React.useCallback(() => { onOpenChange?.(false) }, [onOpenChange])
  const value = React.useMemo<DialogCloseContextValue>(
    () => ({ open, requestClose }),
    [open, requestClose],
  )
  return (
    <DialogCloseContext.Provider value={value}>
      <DialogPrimitive.Root onOpenChange={onOpenChange} open={open} {...props} />
    </DialogCloseContext.Provider>
  )
}
const DialogTrigger = DialogPrimitive.Trigger
const DialogPortal = DialogPrimitive.Portal
const DialogClose = DialogPrimitive.Close

/**
 * 打开的 DialogContent 栈（模块级，**仅登记真正打开的弹窗**）。
 *
 * ⚠️ 为何需要它（Radix 1.1.x 实测）：`DismissableLayer` 只在
 * `index === layers.length - 1` 时才注册 ESC 监听（document capture）。而 Radix
 * 在弹窗关闭后**仍保留 `DialogContent` 挂载**（退出动画走 `Presence`），加上
 * 本应用同时渲染多个弹窗，实测 `layers.length` 可达 7 —— 真正可见的弹窗因此
 * 几乎永远不会是 “最高层”，Radix 自身的 ESC 通路就永久失效（表现为「ESC 完全无法关闭弹窗」）。
 *
 * 这里用自建栈替代 Radix 的 `isHighestLayer`：只有 `open` 为真的层入栈，
 * 其余（含 tooltip 等其它 Radix 浮层）不影响判定。
 *
 * ⚠️ 这是 fork 侧的健壮化补丁（基线 `ui/Dialog.tsx` / `ImportNovelDialog.tsx`
 * 与本库逐字节相同，同样存在该缺陷）；若上游日后自行修复 ESC，需回来核对并收敛本段。
 */
const dialogStack: symbol[] = []

/** 弹窗上下文：受控关闭回调 + 当前是否打开（`open` 未传即非受控，保守视为打开） */
interface DialogCloseContextValue {
  open?: boolean
  requestClose: () => void
}

const DialogCloseContext = React.createContext<DialogCloseContextValue | null>(null)

/** 遮罩层 - 增强的模糊效果 */
const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={cn(
      'fixed inset-0 z-50 bg-black/30 backdrop-blur-sm',
      'data-[state=open]:animate-in data-[state=closed]:animate-out',
      'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
      className
    )}
    style={{
      backdropFilter: 'blur(12px)',
    }}
    {...props}
  />
))
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName

/** 对话框主体 - 增强的阴影和动画 */
const DialogContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(({ className, children, onEscapeKeyDown, ...props }, ref) => {
  const text = useLocaleStore(s => s.text)
  const dialogContext = React.useContext(DialogCloseContext)
  const requestClose = dialogContext?.requestClose
  // `open` 未传（非受控弹窗）时无法感知，保守视为「打开」
  const dialogOpen = dialogContext?.open !== false
  const layerToken = React.useRef<symbol>(Symbol('dialog-layer'))

  // 只登记真正打开的弹窗：Radix 关闭后仍会保留 DialogContent 挂载，
  // 若不加此门禁，已关闭的层会把栈顶占住（实测栈长 7，真正可见的弹窗永远不是栈顶）。
  React.useEffect(() => {
    if (!dialogOpen) return
    const token = layerToken.current
    dialogStack.push(token)
    return () => {
      const index = dialogStack.indexOf(token)
      if (index >= 0) dialogStack.splice(index, 1)
    }
  }, [dialogOpen])

  React.useEffect(() => {
    if (!requestClose || !dialogOpen) return
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      // 只处理最内层「打开」的弹窗（其余层等它关掉后再接手）
      if (dialogStack[dialogStack.length - 1] !== layerToken.current) return
      // 先给消费者拦截机会：`onEscapeKeyDown` 里 `preventDefault()` 即显式禁用 ESC 关闭
      // （如「修稿合并」弹窗——防误触丢失合并进度）
      onEscapeKeyDown?.(event)
      if (event.defaultPrevented) return
      event.preventDefault()
      requestClose()
    }
    // 用 window 的 capture 阶段：早于 Radix 挂在 document 上的监听，
    // 避免被上层浮层消费掉；本组件自己的 dialogStack 保证了嵌套语义
    window.addEventListener('keydown', handleKeyDown, { capture: true })
    return () => window.removeEventListener('keydown', handleKeyDown, { capture: true })
  }, [onEscapeKeyDown, requestClose, dialogOpen])

  return (
  <DialogPortal>
    <DialogOverlay />
    <DialogPrimitive.Content
      ref={ref}
      className={cn(
        'fixed left-[50%] top-[50%] z-50 w-full max-w-lg translate-x-[-50%] translate-y-[-50%]',
        'rounded-2xl outline-none',
        'bg-[var(--color-bg)] border border-[var(--color-border)]',
        'shadow-2xl shadow-black/20',
        'duration-300 ease-out data-[state=open]:animate-in data-[state=closed]:animate-out',
        'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
        'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
        'data-[state=closed]:slide-out-to-left-1/2 data-[state=closed]:slide-out-to-top-[50%]',
        'data-[state=open]:slide-in-from-left-1/2 data-[state=open]:slide-in-from-top-[50%]',
        className
      )}
      style={{
        boxShadow: '0 25px 50px -12px rgba(0, 0, 0, 0.25), 0 0 0 1px rgba(255, 255, 255, 0.05)',
      }}
      // 仍然把 onEscapeKeyDown 交给 Radix：非受控弹窗（无 onOpenChange）走不了
      // 本组件的兑底通道，仍需它自己的 ESC 路径能尊重 `preventDefault()` 的显式opt-out
      onEscapeKeyDown={onEscapeKeyDown}
      {...props}
    >
      {children}
      <DialogPrimitive.Close
        className="absolute right-4 top-4 rounded-md opacity-60 hover:opacity-100 transition-all duration-200 hover:bg-[var(--color-hover)] p-1"
        style={{
          background: 'transparent',
          border: 'none',
          cursor: 'pointer',
          color: 'var(--color-text-muted)',
        }}
      >
        <X size={16} />
        <span className="sr-only">{text('关闭', 'Close')}</span>
      </DialogPrimitive.Close>
    </DialogPrimitive.Content>
  </DialogPortal>
  )
})
DialogContent.displayName = DialogPrimitive.Content.displayName

/** 对话框头部 */
const DialogHeader = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn(
      'px-6 py-4 border-b border-[var(--color-border)]',
      className
    )}
    {...props}
  />
)
DialogHeader.displayName = 'DialogHeader'

/** 对话框底部 */
const DialogFooter = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn(
      'flex items-center justify-between px-6 py-4 border-t border-[var(--color-border)]',
      className
    )}
    {...props}
  />
)
DialogFooter.displayName = 'DialogFooter'

/** 对话框标题 */
const DialogTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn('text-base font-semibold text-[var(--color-text)]', className)}
    {...props}
  />
))
DialogTitle.displayName = DialogPrimitive.Title.displayName

/** 对话框描述 */
const DialogDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn('text-sm text-[var(--color-text-muted)]', className)}
    {...props}
  />
))
DialogDescription.displayName = DialogPrimitive.Description.displayName

export {
  Dialog,
  DialogPortal,
  DialogOverlay,
  DialogClose,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
}
