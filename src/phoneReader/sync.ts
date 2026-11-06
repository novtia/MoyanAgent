/** Payload the main window sends to the phone-reader window. */
export type PhoneReaderPayload = {
  path: string;
  title: string;
  text: string;
  firstLineIndent: boolean;
  paragraphSpacing: boolean;
};

export const PHONE_READER_LABEL = "phone-reader";
export const PHONE_READER_EVENT = "phone-reader:update";
export const PHONE_READER_READY = "phone-reader:ready";
