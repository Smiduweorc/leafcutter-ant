import remote, { colors } from 'https://example.com/lib/colors.js'
import answer from 'https://example.com/lib/typed.ts'
export default { pools: colors.map((color) => ({ name: remote(color), answer })) }
