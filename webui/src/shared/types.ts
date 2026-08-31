export type TranslationParams = Record<string, string | number>;
export type Translator = (value: string, params?: TranslationParams) => string;

export class LocalizedError extends Error {
  constructor(
    readonly key: string,
    readonly params?: TranslationParams
  ) {
    super(key);
    this.name = "LocalizedError";
  }
}

export type ToastItem = {
  id: string;
  message: string;
  tone?: "success" | "error";
};

export type ConfirmRequest = {
  title: string;
  message: string;
  confirmLabel: string;
  action: () => void;
  cancel?: () => void;
  showCancel?: boolean;
};
