"use strict";
const iface = require("./interface.json");
const errors = require("./errors.json");
const versions = require("./versions.json");
const byCode = Object.fromEntries(errors.errors.map((e) => [e.code, e]));
module.exports = { interface: iface, errors: errors.errors, versions, errorByCode: (c) => byCode[c] };
