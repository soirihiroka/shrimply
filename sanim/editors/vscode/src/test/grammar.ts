import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { Registry, parseRawGrammar, INITIAL } from 'vscode-textmate';
import { loadWASM, OnigScanner, OnigString } from 'vscode-oniguruma';

async function main(): Promise<void> {
    await loadWASM(new Uint8Array(await readFile(require.resolve('vscode-oniguruma/release/onig.wasm'))).buffer);
    const grammarPath = path.resolve(__dirname, '../../syntaxes/sanim.tmLanguage.json');
    const registry = new Registry({
        onigLib: Promise.resolve({ createOnigScanner: sources => new OnigScanner(sources), createOnigString: source => new OnigString(source) }),
        loadGrammar: async scope => scope === 'source.sanim' ? parseRawGrammar(await readFile(grammarPath, 'utf8'), grammarPath) : null,
    });
    const grammar = await registry.loadGrammar('source.sanim');
    assert.ok(grammar);
    const tokens = (line: string) => grammar.tokenizeLine(line, INITIAL).tokens.map(token => ({ text: line.slice(token.startIndex, token.endIndex), scopes: token.scopes }));
    const has = (line: string, text: string, scope: string) => tokens(line).some(token => token.text === text && token.scopes.includes(scope));
    assert.ok(has('let duration = 4m1s', '4m1s', 'constant.numeric.duration.sanim'));
    assert.ok(has('let marker = Circle(radius=12pt)', 'Circle', 'support.function.sanim'));
    assert.ok(has('fn draw(): Unit {', 'draw', 'entity.name.function.sanim'));
    assert.ok(has('#3584e4', '#3584e4', 'comment.line.number-sign.sanim'));
    assert.ok(!tokens('Color("#3584e4")').some(token => token.scopes.includes('comment.line.number-sign.sanim')));
    assert.ok(has('# ordinary comment', '# ordinary comment', 'comment.line.number-sign.sanim'));
    assert.ok(tokens('"Value: {round(2.5):04d}"').some(token => token.scopes.includes('meta.interpolation.sanim')));
    assert.ok(!tokens('r"literal {value}"').some(token => token.scopes.includes('meta.interpolation.sanim')));
    assert.ok(!tokens('"literal {{value}}"').some(token => token.scopes.includes('meta.interpolation.sanim')));
    for (const prefix of ['"""', "'''", 'r"""', "r'''"]) {
        let state = grammar.tokenizeLine(`let sql = ${prefix}`, INITIAL).ruleStack;
        const middle = grammar.tokenizeLine('SELECT "name" FROM samples # string text', state);
        assert.ok(middle.tokens.every(token => token.scopes.some(scope => scope.startsWith('string.quoted.triple.'))));
        assert.ok(!middle.tokens.some(token => token.scopes.includes('comment.line.number-sign.sanim')));
        state = middle.ruleStack;
        const interpolation = grammar.tokenizeLine('WHERE name = {name}', state);
        assert.equal(interpolation.tokens.some(token => token.scopes.includes('meta.interpolation.sanim')), !prefix.startsWith('r'));
        const ending = grammar.tokenizeLine(prefix.replace(/^r/, ''), interpolation.ruleStack);
        assert.ok(grammar.tokenizeLine('# real comment', ending.ruleStack).tokens.some(token => token.scopes.includes('comment.line.number-sign.sanim')));
    }
    registry.dispose();
    console.log('Sanim TextMate grammar: declarations, units, colors, comments, raw strings, interpolation passed');
}
void main().catch(error => { console.error(error); process.exitCode = 1; });
