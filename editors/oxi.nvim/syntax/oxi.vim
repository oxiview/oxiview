if exists('b:current_syntax')
  finish
endif

runtime! syntax/rust.vim
syntax keyword oxiDeclaration view style nextgroup=oxiDeclarationName skipwhite
syntax match oxiDeclarationName /\h\w*/ contained
syntax match oxiTag /\%(<\/\?\)\@<=\h\w*\%(::\h\w*\)*/
syntax match oxiProperty /\<\h\w*\ze\s*=\s*{/

highlight default link oxiDeclaration Keyword
highlight default link oxiDeclarationName Function
highlight default link oxiTag Type
highlight default link oxiProperty Identifier
let b:current_syntax = 'oxi'
