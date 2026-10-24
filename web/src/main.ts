import "./tokens.css";
import "./app.css";
import "./shell.css";
import "./work.css";
import "./inspector.css";
import "./chrome.css";
import "./responsive.css";
import { App } from "./app";

const root = document.getElementById("app");
if (root) void new App(root).start();
