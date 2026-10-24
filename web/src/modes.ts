// One controller per mode (D1).

import { browse } from "./browse";
import { changes } from "./changes";
import { find } from "./find";
import type { ViewController } from "./page";
import type { Mode } from "./router";
import { top } from "./top";

export const controllers: Record<Mode, ViewController> = { browse, top, find, changes };
