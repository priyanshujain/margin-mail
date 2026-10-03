import { call, isTauri } from "../ipc";

export interface OtpAutofillStatus {
  available: boolean;
  enabled: boolean;
}

export const otpAutofillStatus = (): Promise<OtpAutofillStatus> =>
  isTauri
    ? call<OtpAutofillStatus>("otp_autofill_status")
    : Promise.resolve({ available: false, enabled: false });

export const otpAutofillEnable = (): Promise<boolean> =>
  isTauri ? call<boolean>("otp_autofill_enable") : Promise.resolve(false);
