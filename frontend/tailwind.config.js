import typography from '@tailwindcss/typography';

/**
 * AT&T brand blue scale: 500 is the primary brand blue (#00A8E0), 600 the brand dark blue (#067AB4).
 * White text needs 600 or darker (#00A8E0 on white is 2.6:1, below WCAG AA).
 */
const attBlue = {
  50: '#eef9fd',
  100: '#d5f1fb',
  200: '#aae3f6',
  300: '#6fcfef',
  400: '#3aa5dc',
  500: '#00a8e0',
  600: '#067ab4',
  700: '#05679a',
  800: '#04537c',
  900: '#033e5d',
  950: '#02263a',
};

/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  darkMode: 'media',
  theme: {
    extend: {
      colors: {
        // The UI's primary (indigo) and info/running (blue) colours both follow the AT&T brand.
        indigo: attBlue,
        blue: attBlue,
        att: {
          ...attBlue,
          DEFAULT: '#00a8e0',
          black: '#000000',
          orange: '#ff7200',
          yellow: '#fcb314',
        },
      },
      fontFamily: {
        sans: [
          'ui-sans-serif',
          'system-ui',
          '-apple-system',
          'Segoe UI',
          'Roboto',
          'Helvetica Neue',
          'Arial',
          'sans-serif',
        ],
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'Monaco', 'Consolas', 'monospace'],
      },
      // The preview must show the agent's Markdown faithfully: the typography plugin's decorative backticks
      // around inline code and quotation marks around blockquotes would look like part of the document.
      typography: {
        DEFAULT: {
          css: {
            'code::before': { content: 'none' },
            'code::after': { content: 'none' },
            'blockquote p:first-of-type::before': { content: 'none' },
            'blockquote p:last-of-type::after': { content: 'none' },
          },
        },
      },
    },
  },
  plugins: [typography],
};
