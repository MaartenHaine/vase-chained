// SPDX-FileCopyrightText: 2025 Florian Müllner <fmuellner@gnome.org>
// SPDX-License-Identifier: MIT OR LGPL-2.1-or-later
//
// The JS lint for vase-gnome: gnome-shell 50.1's own tools/eslint.config.js, cut down to this repo. package.json and
// package-lock.json beside it are gnome-shell 50.1's tools/ files, unchanged, so the lint runs the exact eslint,
// eslint-config-gnome and plugin versions gnome-shell pins. Install them only with
// `npm ci --ignore-scripts` (tools/linux/lint.sh), never on the desktop.

import {defineConfig} from '@eslint/config-helpers';
import gnome from 'eslint-config-gnome';

export default defineConfig([
    gnome.configs.recommended,
    gnome.configs.jsdoc,
    {
        rules: {
            camelcase: ['error', {
                properties: 'never',
            }],
            'consistent-return': 'error',
            'eqeqeq': ['error', 'smart'],
            'key-spacing': ['error', {
                mode: 'minimum',
                beforeColon: false,
                afterColon: true,
            }],
            'prefer-arrow-callback': 'error',
            'prefer-const': ['error', {
                destructuring: 'all',
            }],
            'jsdoc/require-param-description': 'off',
            'jsdoc/require-jsdoc': ['error', {
                exemptEmptyFunctions: true,
                publicOnly: {
                    esm: true,
                },
            }],
        },
    }, {
        files: ['vase-gnome/**/*.js'],
        languageOptions: {
            globals: {
                global: 'readonly',
            },
        },
    },
]);
