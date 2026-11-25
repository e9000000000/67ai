const responseMarkdown = markdownit({
  html: false,
  breaks: true,
  linkify: true,
  highlight(code, language) {
    return language && hljs.getLanguage(language)
      ? hljs.highlight(code, {language, ignoreIllegals:true}).value : '';
  }
});
responseMarkdown.renderer.rules.link_open = (tokens, index, options, env, renderer) => {
  tokens[index].attrSet('target', '_blank');
  tokens[index].attrSet('rel', 'noopener noreferrer');
  return renderer.renderToken(tokens, index, options);
};
function messageBody(message) {
  const formatted = ['assistant', 'reviewer'].includes(message.role);
  return `<div class="message-body${formatted ? ' markdown' : ''}">${formatted ? responseMarkdown.render(message.text) : escape(message.text)}</div>`;
}
