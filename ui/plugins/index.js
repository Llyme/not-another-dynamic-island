// The faces of the native plugins that draw more than the blocks can (the plugin itself is in src/builtin): each file draws one
// plugin's cards and pill. A plugin that is switched off sends nothing, so its file has nothing to draw. Every other plugin
// (a module, a declarative plugin) is drawn by ../pluginui.js, from its data.
import claude from "./claude-code.js";
import downloads from "./downloads.js";
import eyes from "./eyes.js";
import games from "./games.js";
import media from "./media.js";
import pageReader from "./page-reader.js";
import soundLight from "./sound-light.js";
import work from "./work.js";

export const plugins = [claude, games, downloads, media, pageReader, work, eyes, soundLight];
