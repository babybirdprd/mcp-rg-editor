// src/components/ui/use-toast.ts
// Shim for legacy code. Prefer using `import { toast } from "sonner"` directly.
import { toast } from "sonner";

// Accepts a string or a minimal object with title/description/variant for compatibility
export function useToast() {
  return {
    toast: (opts: string | { title?: string; description?: string; variant?: string }) => {
      if (typeof opts === "string") {
        toast(opts);
      } else if (typeof opts === "object") {
        if (opts.variant === "destructive") {
          toast.error(opts.description ?? opts.title ?? "Error");
        } else {
          toast.success(opts.description ?? opts.title ?? "Success");
        }
      }
    },
  };
}
