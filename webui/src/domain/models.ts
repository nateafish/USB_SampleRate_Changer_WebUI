export type PolicySettings = {
  policy: string;
  rate: string;
  customRate: string;
  bitDepth: string;
  drc: boolean;
  forceUsbv2: boolean;
  forceBluetoothQti: boolean;
};

export type ExecResult = {
  code: number;
  stdout: string;
  stderr: string;
};
